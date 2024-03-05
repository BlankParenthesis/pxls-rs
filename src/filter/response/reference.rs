use reqwest::{StatusCode, header};
use warp::{http::Uri, reply::Reply};
use serde::Serialize;

pub trait Referenceable {
	fn location(&self) -> Uri;
}

impl<T: Referenceable> Referenceable for &T {
	fn location(&self) -> Uri { (*self).location() }
}

#[derive(Debug, Serialize)]
pub struct Reference<T: Serialize> {
	#[serde(with = "http_serde::uri")]
	uri: Uri,
	view: T,
}

impl<T: Serialize> Reference<T> {
	pub fn inner(&self) -> &T {
		&self.view
	}
}

impl<T: Serialize + Referenceable> From<T> for Reference<T> {
	fn from(value: T) -> Self {
		Self {
			uri: value.location(),
			view: value,
		}
	}
}

pub fn created<'t, T>(body: &'t T) -> impl Reply
where
	T: Serialize,
	Reference<&'t T>: From<&'t T>,
{
	let reference = Reference::from(body);
	warp::reply::with_header(
		warp::reply::with_status(
			warp::reply::json(&reference),
			StatusCode::CREATED,
		),
		header::LOCATION,
		reference.uri.to_string(),
	)
}