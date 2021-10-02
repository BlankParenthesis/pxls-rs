use actix_web::web::{Bytes, BytesMut};
use actix_web::{FromRequest, HttpMessage, error};
use actix_web::http::header;

use std::pin::Pin;
use std::future::Future;

pub struct BinaryPatch {
	/// Tuple is always ordered.
	/// It always matches the data in length.
	range: Option<(usize, usize)>,
	data: Bytes,
	expected_length: Option<usize>,
}

#[derive(Debug)]
pub enum InvalidPatch {
	EmptyPatch,
	LengthMismatch,
	RangeMisordered,
}

impl std::fmt::Display for InvalidPatch {
	fn fmt(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
		match self {
			InvalidPatch::EmptyPatch => write!(formatter, "Empty Patch"),
			InvalidPatch::LengthMismatch => write!(formatter, "Length Mismatch"),
			InvalidPatch::RangeMisordered => write!(formatter, "Range Misordered"),
		}
	}
}

impl From<InvalidPatch> for error::Error {
	fn from(error: InvalidPatch) -> Self {
		error::ErrorConflict(error)
	}
}

impl BinaryPatch {
	/// Attempt to formulate a new binary patch.
	/// Result in Err if range and data length are mismatched.
	pub fn new(
		range: Option<(u64, u64)>,
		data: Bytes,
		expected_length: Option<u64>,
	) -> Result<Self, InvalidPatch> {
		let range = range.map(|(start, end)| (start as usize, end as usize));
		let expected_length = expected_length.map(|length| length as usize);

		if let Some((start, end)) = range {
			if start == end {
				return Err(InvalidPatch::EmptyPatch);
			}

			if start > end {
				return Err(InvalidPatch::RangeMisordered);
			}

			if end - start != data.len() {
				return Err(InvalidPatch::LengthMismatch);
			}
		}

		Ok(Self {
			range,
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
						BinaryPatch::new(range, data, instance_length)
							.map_err(|e| e.into())
					})
				},
				None => Err(error::ErrorBadRequest("Missing content-range header")),
				_ => Err(error::ErrorBadRequest("Unknown content-range unit")),
			}
		})
	}
}

pub trait Patchable {
	type Patch;

	fn can_patch(&self, patch: &Self::Patch) -> bool;
	fn unchecked_patch(&mut self, patch: &Self::Patch);

	fn patch(&mut self, patch: &Self::Patch) -> Result<(), ()> {
		if self.can_patch(patch) {
			self.unchecked_patch(patch);
			Ok(())
		} else {
			Err(())
		}
	}
}

impl Patchable for BytesMut {
	type Patch = BinaryPatch;

	fn can_patch(&self, patch: &Self::Patch) -> bool {
		if let Some(length) = patch.expected_length {
			if length != self.len() {
				return false;
			}
		}

		match patch.range {
			Some((_, end)) => (end <= self.len()),
			None => (self.len() == patch.data.len()),
		}
	}

	fn unchecked_patch(&mut self, patch: &Self::Patch) {
		if let Some((start, end)) = patch.range {
			let data = &mut self[start..end];
			data.copy_from_slice(&*patch.data);
		} else {
			self.copy_from_slice(&*patch.data);
		}
	}
}