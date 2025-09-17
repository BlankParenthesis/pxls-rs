use std::{
	convert::{Infallible, TryFrom},
	io::Seek,
	fmt::{self, Display, Formatter},
	ops::{Range as OpsRange, RangeFrom as OpsRangeFrom},
	time::SystemTime,
};

use tinyrand::{RandRange, Wyrand, Seeded};
use reqwest::StatusCode;
use thiserror::Error;
use warp::{reject::Reject, Reply, reply, Filter, Rejection};
use warp::hyper::Response;

use crate::{AsyncRead, Len};

use super::*;

#[derive(Error, Debug)]
pub enum RangeIndexError {
	UnknownUnit,
	TooLarge(usize),
}

impl Display for RangeIndexError {
	fn fmt(
		&self,
		f: &mut Formatter<'_>,
	) -> fmt::Result {
		let reason = match self {
			Self::UnknownUnit => "unit not supported",
			Self::TooLarge(_length) => "range exceeds bounds",
		};

		write!(f, "cannot index with range: {}", reason)
	}
}

impl Reject for RangeIndexError {}
impl Reply for RangeIndexError {
	fn into_response(self) -> reply::Response {
		let base_response = match self {
			Self::UnknownUnit => StatusCode::BAD_REQUEST.into_response(),
			Self::TooLarge(length) => {
				Response::builder()
					.status(StatusCode::RANGE_NOT_SATISFIABLE)
					.header(header::CONTENT_RANGE, format!("bytes */{}", length))
					.body("".into())
					.unwrap()
			},
		};

		warp::reply::with_header(base_response, header::ACCEPT_RANGES, "bytes")
			.into_response()
	}
}

#[allow(clippy::enum_variant_names)]
#[derive(Debug, Clone)]
pub enum HttpRange {
	FromStartToEnd(OpsRange<usize>),
	FromStartToLast(OpsRangeFrom<usize>),
	FromEndToLast(usize),
}

impl HttpRange {
	pub fn with_length(
		&self,
		length: usize,
	) -> Result<OpsRange<usize>, RangeIndexError> {
		let range = match self {
			Self::FromEndToLast(from_end) => (length - from_end)..length,
			Self::FromStartToLast(range) => range.start..length,
			Self::FromStartToEnd(range) => range.clone(),
		};

		if range.end <= length {
			Ok(range)
		} else {
			Err(RangeIndexError::TooLarge(length))
		}
	}
}

struct DataRange {
	data: Vec<u8>,
	range: OpsRange<usize>,
	length: usize,
}

#[derive(Error, Debug)]
pub enum RangeOrReadError<E> {
	#[error(transparent)]
	RangeErr(RangeIndexError),
	#[error(transparent)]
	ReadErr(#[from] E),
}

impl<E: Reply> Reply for RangeOrReadError<E> {
	fn into_response(self) -> reply::Response {
		match self {
			RangeOrReadError::RangeErr(e) => e.into_response(),
			RangeOrReadError::ReadErr(e) => e.into_response(),
		}
	}
}

async fn data_ranges<D, E>(
	data: &mut D,
	unit: &str,
	ranges: &[HttpRange],
) -> Result<Vec<DataRange>, RangeOrReadError<E>>
where
	D: AsyncRead<Error = E> + Seek + Len,
	E: Send + Reply,
{
	if !unit.eq("bytes") {
		return Err(RangeOrReadError::RangeErr(RangeIndexError::UnknownUnit));
	}

	let length = data.len();

	let mut ranges = ranges.iter()
		.map(|http_range| http_range.with_length(length))
		.collect::<Result<Vec<_>, _>>()
		.map_err(RangeOrReadError::RangeErr)?;

	ranges.sort_by_key(|range| range.start);

	// gaps smaller than this will be collapsed
	let inbetween_threshold = 32;
	let mut iter = ranges.into_iter();
	// TODO: if ranges are collapsed to a single range,
	// it would be nice to not bother with a multipart response.
	// Clients may not like that though
	let mut efficient_ranges = vec![iter.next().unwrap()];

	for range in iter {
		let current_range = efficient_ranges.last_mut().unwrap();
		let gap = range.start.saturating_sub(current_range.end);

		if gap < inbetween_threshold {
			// extend last range
			current_range.end = range.end;
		} else {
			// start a new range
			efficient_ranges.push(range);
		}
	}

	let mut ranges = Vec::with_capacity(efficient_ranges.len());
	for range in efficient_ranges {
		let length = range.start - range.end;

		let mut subdata: Vec<u8> = std::iter::repeat(0)
			.take(length)
			.collect();

		data.seek(std::io::SeekFrom::Start(
			u64::try_from(range.start).unwrap(),
		))
		.unwrap();
	
		// TODO: assert correct read_size
		let read_size = data.read(&mut subdata).await?;
		debug_assert_eq!(read_size, length);

		ranges.push(DataRange {
			data: subdata,
			range,
			length
		})
	}

	Ok(ranges)
}

fn choose_boundary(datas: &[DataRange]) -> String {
	fn random_boundary_string() -> String {
		let seed = SystemTime::now()
			.duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default()
			.as_millis();
		let mut rand = Wyrand::seed(seed as u64);

		let random_string = std::iter::repeat(())
			.take(8)
			.map(move |_| {
				const ASCII_START: u16 = b'a' as u16;
				const ASCII_END: u16 = b'z' as u16 + 1;
				// get a random lowercase ascii character
				rand.next_range(ASCII_START..ASCII_END) as u8 as char
			})
			.collect::<String>();

		format!("--{}", random_string)
	}

	let mut boundary = random_boundary_string();

	// generate new strings until we get one that doesn't exist in the data
	// NOTE: this is probably not worth the compute cost
	while datas
		.iter()
		.any(|DataRange { data, .. }| {
			data.windows(boundary.len())
				.any(|d| d == boundary.as_bytes())
		}) {
		boundary = random_boundary_string();
	}

	boundary
}

fn merge_ranges(
	datas: &[DataRange],
	boundary: &str,
) -> Vec<u8> {
	let mut joined = Vec::new();

	for DataRange { data, range, length } in datas {
		joined.extend_from_slice(
			format!(
				"{}\r\n\
				content-type: application/octet-stream\r\n\
				content-range: bytes {}-{}/{}\r\n\r\n",
				boundary,
				range.start, range.end, length,
			)
			.as_bytes(),
		);

		joined.extend_from_slice(data);
	}

	joined.extend_from_slice(boundary.as_bytes());

	joined
}

/// rfc9110 section 14
#[derive(Debug, Clone)]
pub enum Range {
	Multi {
		unit: String,
		ranges: Vec<HttpRange>,
	},
	Single {
		unit: String,
		range: HttpRange,
	},
	None,
}

impl TryFrom<&str> for Range {
	type Error = RangeParseError;

	fn try_from(header: &str) -> Result<Self, Self::Error> {
		let (unit, range_data) = header
			.split_once('=')
			.ok_or(RangeParseError::MissingUnit)?;
		
		let unit = String::from(unit);
		let mut ranges: Vec<HttpRange> = range_data
			.split(',')
			.map(HttpRange::try_from)
			.collect::<Result<_, _>>()?;

		match ranges.len() {
			0 => Err(RangeParseError::NoRange),
			1 => {
				Ok(Self::Single {
					unit,
					range: ranges.swap_remove(0),
				})
			},
			_ => Ok(Self::Multi { unit, ranges }),
		}
	}
}

impl TryFrom<&str> for HttpRange {
	type Error = RangeParseError;

	fn try_from(range: &str) -> Result<Self, Self::Error> {
		let tuple = range
			.split_once('-')
			.ok_or_else(|| RangeParseError::MissingHyphenMinus(String::from(range)))?;
		let http_range = match tuple {
			("", "") => Err(RangeParseError::RangeEmpty),
			("", since_end) => {
				since_end.parse()
					.map(HttpRange::FromEndToLast)
					.map_err(RangeParseError::ValueParseError)
			},
			(start, "") => {
				start.parse()
					.map(|start| HttpRange::FromStartToLast(start..))
					.map_err(RangeParseError::ValueParseError)
			},
			(start, end) => {
				start.parse()
					.and_then(|start| {
						end.parse()
							.map(|end| HttpRange::FromStartToEnd(start..end))
					})
					.map_err(RangeParseError::ValueParseError)
			},
		}?;

		if let HttpRange::FromStartToEnd(range) = &http_range {
			if range.end < range.start {
				Err(RangeParseError::Backwards)
			} else {
				Ok(http_range)
			}
		} else {
			Ok(http_range)
		}
	}
}

impl Range {
	pub async fn respond_with<D, E>(
		&self,
		data: &mut D,
	) -> Result<reply::Response, RangeOrReadError<E>>
	where
		D: AsyncRead<Error = E> + Seek + Len,
		E: Send + Reply,
	{
		match self {
			Self::Multi { unit, ranges } => {
				data_ranges(data, unit, ranges).await
					.map(|datas| {
						let boundary = choose_boundary(&datas);
						let merged = merge_ranges(&datas, &boundary);

						let content_type = format!("multipart/byteranges; boundary={}", boundary);

						Response::builder()
							.status(StatusCode::PARTIAL_CONTENT)
							.header(header::CONTENT_TYPE, content_type)
							.body(merged)
							.into_response()
					})
			},
			Self::Single { unit, range } => {
				let range = range
					.with_length(data.len())
					.and_then(|ranges| {
						if unit.eq("bytes") {
							Ok(ranges)
						} else {
							Err(RangeIndexError::UnknownUnit)
						}
					})
					.map_err(RangeOrReadError::RangeErr)?;

				let length = range.end - range.start;
				let mut buffer = vec![0; length];

				data.seek(std::io::SeekFrom::Start(
					u64::try_from(range.start).unwrap(),
				))
				.unwrap();

				data.read(&mut buffer).await
					.map(|read_size| {
						debug_assert_eq!(read_size, length);
						let range = format!("bytes {}-{}/{}", range.start, range.end, data.len());

						Response::builder()
							.status(StatusCode::PARTIAL_CONTENT)
							.header(header::CONTENT_TYPE, "application/octet-stream")
							.header(header::CONTENT_RANGE, range)
							.body(buffer)
							.into_response()
					})
					.map_err(RangeOrReadError::ReadErr)
			},
			Self::None => {
				let length = data.len();
				let mut buffer = vec![0; length];

				data.read(&mut buffer).await
					.map(|read_size| {
						debug_assert_eq!(read_size, length);
						Response::builder()
							.header(header::CONTENT_TYPE, "application/octet-stream")
							.header(header::ACCEPT_RANGES, "bytes")
							.body(buffer)
							.into_response()
					})
					.map_err(RangeOrReadError::ReadErr)
			},
		}
	}
}

impl From<Option<Range>> for Range {
	fn from(option: Option<Range>) -> Self {
		match option {
			None => Range::None,
			Some(value) => value,
		}
	}
}

pub fn default() -> impl Filter<Extract = (Range,), Error = Infallible> + Copy {
	warp::any().and_then(|| async { Result::<_, Infallible>::Ok(Range::None) })
}

pub fn range() -> impl Filter<Extract = (Range,), Error = Rejection> + Copy {
	warp::header(header::RANGE.as_str())
	.and_then(|header: String| async move {
		Range::try_from(header.as_str())
			.map_err(warp::reject::custom)
	})
}
