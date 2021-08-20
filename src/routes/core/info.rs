use actix_web::{get, HttpResponse};
use serde::Serialize;

#[derive(Serialize)]
pub struct ServerInfo {
	name: Option<&'static str>,
	version: Option<&'static str>,
	source: Option<&'static str>,
	extensions: Vec<&'static str>,
}

const SERVER_INFO: ServerInfo = ServerInfo {
	name: Some("unnamed-newpxls-rs"),
	version: Some(env!("CARGO_PKG_VERSION")),
	source: Some(env!("CARGO_PKG_REPOSITORY")),
	extensions: vec![],
};

guard!(InfoAccess, Info);

#[get("/info")]
pub async fn info(_access: InfoAccess) -> HttpResponse {
	HttpResponse::Ok()
		.json(SERVER_INFO)
}