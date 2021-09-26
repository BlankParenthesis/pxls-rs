use actix_web::{get, HttpResponse};
use serde::Serialize;

use crate::objects::User;
use crate::access::permissions::Permission;

#[derive(Serialize)]
pub struct ServerInfo {
	name: Option<&'static str>,
	version: Option<&'static str>,
	source: Option<&'static str>,
	extensions: &'static [&'static str],
}

const SERVER_INFO: ServerInfo = ServerInfo {
	name: Some("unnamed-newpxls-rs"),
	version: Some(env!("CARGO_PKG_VERSION")),
	source: Some(env!("CARGO_PKG_REPOSITORY")),
	extensions: &["authentication"],
};

guard!(InfoAccess, Info);

#[get("/info")]
pub async fn info(user: Option<User>) -> HttpResponse {
	if user.unwrap_or_default().permissions.contains(&Permission::Info) {
		HttpResponse::Ok().json(SERVER_INFO)
	} else {
		actix_web::error::ErrorForbidden("Missing Permissions").into()
	}
}