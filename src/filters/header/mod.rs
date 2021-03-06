use http::header;

use super::*;

pub mod accept_encoding;
pub mod authorization;
pub mod content_range;
pub mod range;

#[derive(Debug)]
pub enum RangeParseError {
	MissingUnit,
	MissingSize,
	MissingHyphenMinus(String),
	ValueParseError(ParseIntError),
	RangeEmpty,
	NoRange,
	Backwards,
}

impl Reject for RangeParseError {}

impl Reply for RangeParseError {
	fn into_response(self) -> reply::Response {
		Response::builder()
			.status(StatusCode::BAD_REQUEST)
			.header(header::ACCEPT_RANGES, "bytes")
			.body("".into())
			.unwrap()
	}
}
