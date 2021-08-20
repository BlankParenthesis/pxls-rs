use rocket::serde::json::{json, Value};
use serde::Serialize;

use crate::access::permissions;

guard!(InfoAccess, Info);

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

#[get("/info")]
pub fn info(_access: InfoAccess) -> Value {
	json!(SERVER_INFO)
}