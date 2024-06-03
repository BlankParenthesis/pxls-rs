use reqwest::{StatusCode, header};
use warp::{http::Uri, reply::Reply};
use serde::Serialize;

#[derive(Debug, Serialize, Clone)]
pub struct Reference<T: Serialize> {
	#[serde(with = "http_serde::uri")]
	uri: Uri,
	view: T,
}

impl<T: Serialize> Reference<T> {
	pub fn new(uri: Uri, view: T) -> Self {
		Self { uri, view }
	}

	pub fn created(&self) -> warp::reply::Response {
		let data = warp::reply::json(&self.view);
		warp::reply::with_header(
			warp::reply::with_status(data, StatusCode::CREATED),
			header::LOCATION,
			self.uri.to_string(),
		).into_response()
	}

	pub fn reply(&self) -> warp::reply::Response {
		warp::reply::json(self).into_response()
	}

	pub fn deref(&self) -> warp::reply::Response {
		warp::reply::json(&self.view).into_response()
	}
}
