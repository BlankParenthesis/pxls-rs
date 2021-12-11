use super::*;
use http::header;

pub mod authorization;
pub mod range;
pub mod content_range;

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