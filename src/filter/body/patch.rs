use bytes::Bytes;
use reqwest::{StatusCode, header};
use warp::{reject::Reject, Reply, reply::Response, Filter, Rejection};

use crate::filter::header::content_range::{self, ContentRange};

#[derive(Debug)]
pub enum InvalidPatch {
	EmptyPatch,
	LengthMismatch,
	RangeMisordered,
	BoundsExceeded,
	UnexpectedSize,
	UnknownUnit,
}

impl Reject for InvalidPatch {}

impl Reply for InvalidPatch {
	fn into_response(self) -> Response {
		StatusCode::BAD_REQUEST.into_response()
	}
}

pub struct BinaryPatch {
	pub start: usize,
	pub data: Bytes,
	pub expected_length: Option<usize>,
}

impl BinaryPatch {
	/// Attempt to formulate a new binary patch.
	/// Result in Err if range and data length are mismatched.
	pub fn new(
		data: Bytes,
		content_range: ContentRange,
	) -> Result<Self, InvalidPatch> {
		let expected_length = content_range.size;

		if let Some((start, end)) = content_range.range {
			if start == end {
				return Err(InvalidPatch::EmptyPatch);
			}

			if start > end {
				return Err(InvalidPatch::RangeMisordered);
			}

			if (end - start) != data.len() {
				return Err(InvalidPatch::LengthMismatch);
			}
		}

		let start = content_range
			.range
			.map(|(start, _end)| start)
			.unwrap_or(0);

		Ok(Self {
			start,
			data,
			expected_length,
		})
	}
}

// TODO: size limit
pub fn bytes() -> impl Filter<Extract = (BinaryPatch,), Error = Rejection> + Copy {
	warp::patch()
		.and(warp::body::bytes())
		.and(warp::header::exact(
			header::CONTENT_TYPE.as_str(),
			"application/octet-stream",
		))
		.and(content_range::content_range())
		.and_then(|bytes, range| async move {
			BinaryPatch::new(bytes, range).map_err(warp::reject::custom)
		})
}

// TODO: multipart patch?
