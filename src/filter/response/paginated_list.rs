use serde::{Deserialize, Serialize, Serializer};
use warp::http::Uri;

fn optional_uri<S>(uri: &Option<Uri>, ser: S) -> Result<S::Ok, S::Error>
where S: Serializer {
	if let Some(uri) = uri {
		http_serde::uri::serialize(uri, ser)
	} else {
		ser.serialize_none()
	}
}

#[derive(Serialize, Debug)]
pub struct Page<T> {
	pub items: Vec<T>,
	#[serde(serialize_with = "optional_uri")]
	pub next: Option<Uri>,
	// TODO: either find some magical way to generate this or change the spec
	#[serde(serialize_with = "optional_uri")]
	pub previous: Option<Uri>,
}

#[derive(Deserialize, Debug)]
pub struct PaginationOptions<T: PageToken> {
	#[serde(default)]
	pub page: T,
	pub limit: Option<usize>,
}

pub trait PageToken: Default {
	fn start() -> Self { Self::default() }
}
