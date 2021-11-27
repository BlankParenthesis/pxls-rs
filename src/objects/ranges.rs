use std::{
	convert::TryFrom,
	fmt::{self, Display, Formatter},
	io::{Read, Seek},
	num::ParseIntError,
	ops::{Range, RangeFrom},
};

use actix_web::{dev::Payload, error, http::header, FromRequest, HttpRequest, HttpResponse};
use futures_util::future::{ready, Ready};
use http::StatusCode;
use rand::{self, Rng};

#[derive(Debug)]
pub enum RangeParseError {
	NotAscii,
	MissingUnit,
	MissingHyphenMinus(String),
	ValueParseError(ParseIntError),
	RangeEmpty,
	NoRange,
	Backwards,
}

impl Display for RangeParseError {
	fn fmt(
		&self,
		f: &mut Formatter<'_>,
	) -> fmt::Result {
		write!(
			f,
			"invalid range format: {}",
			match self {
				Self::NotAscii => "header contained non-ascii characters",
				Self::MissingUnit => "missing unit",
				Self::MissingHyphenMinus(_range) => "missing hyphen-minus",
				Self::ValueParseError(_err) => "value parse error",
				Self::RangeEmpty => "empty range",
				Self::NoRange => "no ranges",
				Self::Backwards => "range ended before beginning",
			}
		)
	}
}

impl From<RangeParseError> for error::Error {
	fn from(error: RangeParseError) -> Self {
		HttpResponse::build(StatusCode::BAD_REQUEST)
			.header("accept-ranges", "bytes")
			.body(error.to_string())
			.into()
	}
}

#[derive(Debug)]
pub enum RangeIndexError {
	UnknownUnit(usize),
	MultiUnsupported(usize),
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
				Self::UnknownUnit(_length) => "unit not supported",
				Self::MultiUnsupported(_length) => "multi ranges not supported",
				Self::TooLarge(_length) => "range exceeds bounds",
			}
		)
	}
}

impl From<RangeIndexError> for error::Error {
	fn from(error: RangeIndexError) -> Self {
		if let RangeIndexError::UnknownUnit(_) = error {
			HttpResponse::build(StatusCode::BAD_REQUEST)
				.header("accept-ranges", "bytes")
				.finish()
				.into()
		} else {
			let length = match error {
				RangeIndexError::UnknownUnit(length) => length,
				RangeIndexError::MultiUnsupported(length) => length,
				RangeIndexError::TooLarge(length) => length,
			};

			let range_spec = header::ContentRangeSpec::Bytes {
				range: None,
				instance_length: Some(length as u64),
			};

			HttpResponse::build(StatusCode::RANGE_NOT_SATISFIABLE)
				.header("accept-ranges", "bytes")
				.header("content-range", range_spec)
				.finish()
				.into()
		}
	}
}

pub enum HttpRange {
	FromStartToEnd(Range<usize>),
	FromStartToLast(RangeFrom<usize>),
	FromEndToLast(usize),
}

impl HttpRange {
	fn with_length(
		&self,
		length: usize,
	) -> Result<Range<usize>, RangeIndexError> {
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

pub enum RangeHeader {
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

struct DataRange {
	data: Vec<u8>,
	range: Range<usize>,
}

fn data_ranges<D>(
	data: &mut D,
	unit: &str,
	ranges: &[HttpRange],
) -> Result<Vec<DataRange>, error::Error>
where
	D: Read + Seek + crate::objects::sector_cache::Len,
{
	if !unit.eq("bytes") {
		return Err(RangeIndexError::UnknownUnit(data.len()).into());
	}

	let mut ranges = ranges
		.iter()
		.map(|http_range| http_range.with_length(data.len()))
		.collect::<Result<Vec<_>, _>>()
		.map_err(error::Error::from)?;

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

impl RangeHeader {
	pub fn respond_with<D>(
		&self,
		data: &mut D,
	) -> HttpResponse
	where
		D: Read + Seek + crate::objects::sector_cache::Len,
	{
		match self {
			Self::Multi { unit, ranges } => {
				match data_ranges(data, unit, ranges) {
					Ok(datas) => {
						let boundary = choose_boundary(&datas);
						let merged = merge_ranges(&datas, &boundary);

						HttpResponse::build(StatusCode::PARTIAL_CONTENT)
							.content_type(format!("multipart/byteranges; boundary={}", boundary))
							.body(merged)
					},
					Err(error) => error.into(),
				}
			},
			Self::Single { unit, range } => {
				let result = range
					.with_length(data.len())
					.map_err(error::Error::from)
					.and_then(|ranges| {
						if unit.eq("bytes") {
							Ok(ranges)
						} else {
							Err(RangeIndexError::UnknownUnit(data.len()).into())
						}
					});

				match result {
					Ok(range) => {
						HttpResponse::build(StatusCode::PARTIAL_CONTENT)
							.content_type("application/octet-stream")
							.header(
								"content-range",
								header::ContentRangeSpec::Bytes {
									range: Some((range.start as u64, range.end as u64)),
									instance_length: Some(data.len() as u64),
								},
							)
							.body({
								let length = range.end - range.start;
								let mut buffer = vec![0; length];

								data.seek(std::io::SeekFrom::Start(
									u64::try_from(range.start).unwrap(),
								))
								.unwrap();

								data.read_exact(&mut buffer).unwrap();

								buffer
							})
					},
					Err(error) => error.into(),
				}
			},
			Self::None => {
				HttpResponse::build(StatusCode::OK)
					.content_type("application/octet-stream")
					.header("accept-ranges", "bytes")
					.body({
						let length = data.len();
						let mut buffer = vec![0; length];

						data.read_exact(&mut buffer).unwrap();

						buffer
					})
			},
		}
	}
}

impl From<Option<RangeHeader>> for RangeHeader {
	fn from(option: Option<RangeHeader>) -> Self {
		match option {
			None => RangeHeader::None,
			Some(value) => value,
		}
	}
}

impl FromRequest for RangeHeader {
	type Config = ();
	type Error = RangeParseError;
	type Future = Ready<Result<Self, Self::Error>>;

	fn from_request(
		req: &HttpRequest,
		_: &mut Payload,
	) -> Self::Future {
		ready(
			req.headers()
				.get(http::header::RANGE)
				.map(|header| {
					let header = header
						.to_str()
						.map_err(|_| RangeParseError::NotAscii)?;

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
				})
				.unwrap_or(Ok(RangeHeader::None)),
		)
	}
}
