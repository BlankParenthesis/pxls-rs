use std::num::ParseIntError;
use std::ops::{Range, RangeFrom, RangeFull, Index};
use std::fmt::{self, Display, Formatter};
use actix_web::{FromRequest, HttpRequest, dev::{Payload, HttpResponseBuilder}, error, web::{BytesMut, Bytes}, HttpResponse};
use actix_web::http::header;
use futures_util::future::{Ready, ready};
use http::StatusCode;
use core::borrow::Borrow;

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
	fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
		write!(f, "invalid range format: {}", match self {
			Self::NotAscii => "header contained non-ascii characters",
			Self::MissingUnit => "missing unit",
			Self::MissingHyphenMinus(range) => "missing hyphen-minus",
			Self::ValueParseError(err) => "value parse error",
			Self::RangeEmpty => "empty range",
			Self::NoRange => "no ranges",
			Self::Backwards => "range ended before beginning",
		})
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
	fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
		write!(f, "cannot index with range: {}", match self {
			Self::UnknownUnit(_length) => "unit not supported",
			Self::MultiUnsupported(_length) => "multi ranges not supported",
			Self::TooLarge(_length) => "range exceeds bounds",
		})
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
				instance_length: Some(length as u64)
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
	fn with_length(&self, length: usize) -> Result<Range<usize>, RangeIndexError> {
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

impl RangeHeader {
	pub fn respond_with(&self, data: &BytesMut) -> HttpResponse {
		match self {
			Self::Multi { unit, ranges } => {
				// TODO: sort, eliminate small gaps
				let result = ranges.iter()
					.map(|http_range| http_range.with_length(data.len()))
					.collect::<Result<Vec<_>, _>>()
					.map(|ranges| {
						// TODO: compute capacity
						let mut joined = Vec::new();
						
						// TODO: select a valid boundary (and also use it later in the multipart content type)
						let boundary = "--hey, red";

						for range in ranges {
							joined.extend_from_slice(boundary.as_bytes());
							joined.extend_from_slice(b"\r\n");
							joined.extend_from_slice(b"content-type: application/octet-stream");
							joined.extend_from_slice(b"\r\n");
							joined.extend_from_slice(format!(
								"content-range: bytes {}-{}/{}",
								range.start,
								range.end,
								data.len()
							).as_bytes());
							joined.extend_from_slice(b"\r\n");
							joined.extend_from_slice(b"\r\n");
							joined.extend_from_slice(&data[range]);
						}
						joined.extend_from_slice(boundary.as_bytes());
						
						joined
					})
					.map_err(error::Error::from)
					.and_then(|ranges| {
						// TODO: might be nicer to check this first
						if unit.eq("bytes") {
							Ok(ranges)
						} else {
							Err(RangeIndexError::UnknownUnit(data.len()).into())
						}
					});

				match result {
					Ok(data) => {
						let boundary = "hey, red";

						HttpResponse::build(StatusCode::PARTIAL_CONTENT)
							.content_type(format!("multipart/byteranges; boundary={}", boundary))
							.body(data)
					},
					Err(error) => error.into(),
				}
			},
			Self::Single { unit, range } => {
				let result = range.with_length(data.len())
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
							.header("content-range", header::ContentRangeSpec::Bytes {
								range: Some((range.start as u64, range.end as u64)),
								instance_length: Some(data.len() as u64)
							})
							.body(Vec::from(&data[range]))
					},
					Err(error) => error.into(),
				}
			},
			Self::None => HttpResponse::build(StatusCode::OK)
				.content_type("application/octet-stream")
				.header("accept-ranges", "bytes")
				.body(Vec::from(data.as_ref())),
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
	type Error = RangeParseError;
	type Future = Ready<Result<Self, Self::Error>>;
	type Config = ();

	fn from_request(req: &HttpRequest, _: &mut Payload) -> Self::Future {
		ready(req.headers().get(http::header::RANGE)
			.map(|header| {
				let header = header.to_str()
					.map_err(|_| RangeParseError::NotAscii)?;

				let (unit, range_data) = header
					.split_once('=')
					.ok_or(RangeParseError::MissingUnit)?;
				let unit = String::from(unit);
		
				let mut ranges: Vec<HttpRange> = range_data.split(',')
					.map(|range| {
						let tuple = range.split_once('-').ok_or_else(|| {
							RangeParseError::MissingHyphenMinus(String::from(range))
						})?;
						let http_range = match tuple {
							("", "") => Err(RangeParseError::RangeEmpty),
							("", since_end) => since_end.parse()
								.map(HttpRange::FromEndToLast)
								.map_err(RangeParseError::ValueParseError),
							(start, "") => start.parse()
								.map(|start| HttpRange::FromStartToLast(start..))
								.map_err(RangeParseError::ValueParseError),
							(start, end) => start.parse()
								.and_then(|start| end.parse()
								.map(|end| HttpRange::FromStartToEnd(start..end)))
								.map_err(RangeParseError::ValueParseError),
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
					1 => Ok(Self::Single { unit, range: ranges.swap_remove(0) }),
					_ => Ok(Self::Multi { unit, ranges }),
				}
			})
			.unwrap_or(Ok(RangeHeader::None))
		)
	}
}
