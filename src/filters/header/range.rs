use super::*;

use std::{
	convert::{TryFrom, Infallible},
	fmt::{self, Display, Formatter},
	io::{Read, Seek},
	ops::{Range as OpsRange, RangeFrom as OpsRangeFrom},
};

use rand::{self, Rng};

#[derive(Debug)]
pub enum RangeIndexError {
	UnknownUnit,
	TooLarge(usize),
}

impl Display for RangeIndexError {
	fn fmt(
		&self,
		f: &mut Formatter<'_>,
	) -> fmt::Result {
		write!(
			f,
			"cannot index with range: {}",
			match self {
				Self::UnknownUnit => "unit not supported",
				Self::TooLarge(_length) => "range exceeds bounds",
			}
		)
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
			}
		};

		warp::reply::with_header(base_response, header::ACCEPT_RANGES, "bytes").into_response()
	}
}

pub enum HttpRange {
	FromStartToEnd(OpsRange<usize>),
	FromStartToLast(OpsRangeFrom<usize>),
	FromEndToLast(usize),
}

impl HttpRange {
	fn with_length(
		&self,
		length: usize,
	) -> Result<OpsRange<usize>, RangeIndexError> {
		let range = match self {
			Self::FromEndToLast(from_end) => length - from_end..length,
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
}

fn data_ranges<D>(
	data: &mut D,
	unit: &str,
	ranges: &[HttpRange],
) -> Result<Vec<DataRange>, RangeIndexError>
where
	D: Read + Seek + crate::objects::sector_cache::Len,
{
	if !unit.eq("bytes") {
		return Err(RangeIndexError::UnknownUnit);
	}

	let mut ranges = ranges
		.iter()
		.map(|http_range| http_range.with_length(data.len()))
		.collect::<Result<Vec<_>, _>>()?;

	ranges.sort_by_key(|range| range.start);

	// gaps smaller than this will be collapsed
	let inbetween_threshold = 32;
	let mut iter = ranges.into_iter();
	// TODO: if ranges are collapsed to a single range,
	// it would be nice to not bother with a multipart response.
	// Clients may not like that though
	let mut efficient_ranges = vec![iter.next().unwrap()];

	for range in iter {
		let mut current_range = efficient_ranges.last_mut().unwrap();

		if (range
			.start
			.saturating_sub(current_range.end))
			< inbetween_threshold
		{
			// extend last range
			current_range.end = range.end;
		} else {
			// start a new range
			efficient_ranges.push(range);
		}
	}

	Ok(efficient_ranges
		.into_iter()
		.map(|range| {
			let length = range.end - range.start;
			let mut subdata: Vec<u8> = std::iter::repeat(0)
				.take(length)
				.collect();

			data.seek(std::io::SeekFrom::Start(
				u64::try_from(range.start).unwrap(),
			))
			.unwrap();
			data.read_exact(&mut subdata).unwrap();

			DataRange {
				data: subdata,
				range,
			}
		})
		.collect())
}

fn choose_boundary(datas: &[DataRange]) -> String {
	fn random_boundary_string() -> String {
		format!(
			"--{}",
			rand::thread_rng()
				.sample_iter::<char, _>(rand::distributions::Standard)
				.take(8)
				.collect::<String>()
		)
	}

	let mut boundary = random_boundary_string();

	// generate new strings until we get one that doesn't exist in the data
	while datas
		.iter()
		.any(|DataRange { data, range: _ }| {
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

	for DataRange { data, range } in datas {
		joined.extend_from_slice(
			format!(
				"{}\r\n\
			content-type: application/octet-stream\r\n\
			content-range: bytes {}-{}/{}\r\n\
			\r\n",
				boundary,
				range.start,
				range.end,
				data.len(),
			)
			.as_bytes(),
		);

		joined.extend_from_slice(data);
	}

	joined.extend_from_slice(boundary.as_bytes());

	joined
}


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
			.map(|range| {
				let tuple = range.split_once('-').ok_or_else(|| {
					RangeParseError::MissingHyphenMinus(String::from(range))
				})?;
				let http_range = match tuple {
					("", "") => Err(RangeParseError::RangeEmpty),
					("", since_end) => {
						since_end
							.parse()
							.map(HttpRange::FromEndToLast)
							.map_err(RangeParseError::ValueParseError)
					},
					(start, "") => {
						start
							.parse()
							.map(|start| HttpRange::FromStartToLast(start..))
							.map_err(RangeParseError::ValueParseError)
					},
					(start, end) => {
						start
							.parse()
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
			})
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

impl Range {
	pub fn respond_with<D>(
		&self,
		data: &mut D,
	) -> reply::Response
	where
		D: Read + Seek + crate::objects::sector_cache::Len,
	{
		match self {
			Self::Multi { unit, ranges } => {
				match data_ranges(data, unit, ranges) {
					Ok(datas) => {
						let boundary = choose_boundary(&datas);
						let merged = merge_ranges(&datas, &boundary);

						Response::builder()
							.status(StatusCode::PARTIAL_CONTENT)
							.header(header::CONTENT_TYPE, format!("multipart/byteranges; boundary={}", boundary))
							.body(merged.into())
							.unwrap()
					},
					Err(error) => error.into_response(),
				}
			},
			Self::Single { unit, range } => {
				let result = range
					.with_length(data.len())
					.and_then(|ranges| {
						if unit.eq("bytes") {
							Ok(ranges)
						} else {
							Err(RangeIndexError::UnknownUnit)
						}
					});

				match result {
					Ok(range) => {
						let length = range.end - range.start;
						let mut buffer = vec![0; length];

						data.seek(std::io::SeekFrom::Start(
							u64::try_from(range.start).unwrap(),
						))
						.unwrap();

						data.read_exact(&mut buffer).unwrap();

						let mut response = buffer.into_response();
						response = reply::with_status(response, StatusCode::PARTIAL_CONTENT).into_response();
						response = reply::with_header(response, header::CONTENT_TYPE, "application/octet-stream").into_response();
						response = reply::with_header(response, header::CONTENT_TYPE, format!("bytes {}-{}/{}", range.start, range.end, length)).into_response();
						response
					},
					Err(error) => error.into_response(),
				}
			},
			Self::None => {
				let length = data.len();
				let mut buffer = vec![0; length];

				data.read_exact(&mut buffer).unwrap();

				Response::builder()
					.header(header::CONTENT_TYPE, "application/octet-stream")
					.header(header::ACCEPT_RANGES, "bytes")
					.body(buffer.into())
					.unwrap()
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
	warp::any()
		.and_then(|| async {
			Result::<_, Infallible>::Ok(Range::None)
		})
}

pub fn range() -> impl Filter<Extract = (Range,), Error = Rejection> + Copy {
	warp::header(header::RANGE.as_str()).and_then(|header: String| async move {
		Range::try_from(header.as_str())
			.map_err(warp::reject::custom)
	})
}