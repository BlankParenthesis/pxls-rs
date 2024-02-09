pub mod accept_encoding;
/// Note: this is actually authentication.
/// It is named authorization because of the http header.
pub mod authorization;
// TODO: both range things here might need re-evaluating in structure
pub mod content_range;
pub mod range;

use std::num::ParseIntError;

use reqwest::StatusCode;
use warp::{http::header, reject::Reject, Reply, reply, hyper::Response};

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
