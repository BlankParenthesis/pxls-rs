use http::Uri;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct Reference<'t, T>
where
	T: Serialize,
{
	#[serde(with = "http_serde::uri")]
	pub uri: Uri,
	pub view: &'t T,
}
