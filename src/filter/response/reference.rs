use warp::http::Uri;
use serde::Serialize;

// TODO: impl Into<Reference> for a bunch of things
#[derive(Debug, Serialize)]
pub struct Reference<'t, T>
where
	T: Serialize,
{
	#[serde(with = "http_serde::uri")]
	pub uri: Uri,
	pub view: &'t T,
}
