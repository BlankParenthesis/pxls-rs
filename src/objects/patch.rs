use std::{convert::TryFrom, future::Future, pin::Pin};

use actix_web::{error, http::header, web::Bytes, FromRequest, HttpMessage};

#[derive(Debug)]
pub enum InvalidPatch {
	EmptyPatch,
	LengthMismatch,
	RangeMisordered,
	BoundsExceeded,
	UnexpectedSize,
}

impl std::fmt::Display for InvalidPatch {
	fn fmt(
		&self,
		formatter: &mut std::fmt::Formatter,
	) -> std::fmt::Result {
		match self {
			InvalidPatch::EmptyPatch => {
				write!(formatter, "Empty Patch")
			},
			InvalidPatch::LengthMismatch => {
				write!(formatter, "Length Mismatch")
			},
			InvalidPatch::RangeMisordered => {
				write!(formatter, "Range Misordered")
			},
			InvalidPatch::BoundsExceeded => {
				write!(formatter, "Patch exceeds object bounds")
			},
			InvalidPatch::UnexpectedSize => {
				write!(formatter, "Expected length mismatch")
			},
		}
	}
}

impl From<InvalidPatch> for error::Error {
	fn from(error: InvalidPatch) -> Self {
		error::ErrorConflict(error)
	}
}

pub struct BinaryPatch {
	pub start: u64,
	pub data: Bytes,
	pub expected_length: Option<usize>,
}

impl BinaryPatch {
	/// Attempt to formulate a new binary patch.
	/// Result in Err if range and data length are mismatched.
	pub fn new(
		range: Option<(u64, u64)>,
		data: Bytes,
		expected_length: Option<u64>,
	) -> Result<Self, InvalidPatch> {
		let expected_length = expected_length.map(|length| length as usize);

		if let Some((start, end)) = range {
			if start == end {
				return Err(InvalidPatch::EmptyPatch);
			}

			if start > end {
				return Err(InvalidPatch::RangeMisordered);
			}

			if usize::try_from(end - start).unwrap() != data.len() {
				return Err(InvalidPatch::LengthMismatch);
			}
		}

		let start = range
			.map(|(start, _end)| start)
			.unwrap_or(0);

		Ok(Self {
			start,
			data,
			expected_length,
		})
	}
}

// TODO: multipart patch
impl FromRequest for BinaryPatch {
	type Config = ();
	type Error = error::Error;
	type Future = Pin<Box<dyn Future<Output = Result<Self, Self::Error>>>>;

	fn from_request(
		request: &actix_web::HttpRequest,
		payload: &mut actix_web::dev::Payload,
	) -> Self::Future {
		let content_type = request.get_header::<header::ContentType>();
		let content_range = request.get_header::<header::ContentRange>();

		let bytes_future = Bytes::from_request(request, payload);

		Box::pin(async move {
			content_type
				.ok_or_else(|| error::ErrorBadRequest("Missing Content Type"))
				.and_then(|header::ContentType(mime)| {
					match mime.essence_str() {
						"application/octet-stream" => Ok(()),
						_ => Err(error::ErrorUnsupportedMediaType("")),
					}
				})?;

			match content_range {
				Some(header::ContentRange(header::ContentRangeSpec::Bytes {
					range,
					instance_length,
				})) => {
					bytes_future.await.and_then(|data| {
						BinaryPatch::new(range, data, instance_length).map_err(|e| e.into())
					})
				},
				None => Err(error::ErrorBadRequest("Missing content-range header")),
				_ => Err(error::ErrorBadRequest("Unknown content-range unit")),
			}
		})
	}
}
