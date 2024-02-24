use reqwest::{StatusCode, header};
use warp::{http::Uri, reply::Reply};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct Reference<'t, T>
where
	T: Serialize,
{
	#[serde(with = "http_serde::uri")]
	uri: Uri,
	view: &'t T,
}

impl<'t, T> From<&'t T> for Reference<'t, T> 
where
	T: Serialize,
	Uri: From<&'t T>,
{
	fn from(value: &'t T) -> Self {
		Self {
			uri: Uri::from(value),
			view: value,
		}
	}
}

pub fn created<'t, T>(body: &'t T) -> impl Reply
where
	T: Serialize,
	Reference<'t, T>: From<&'t T>,
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