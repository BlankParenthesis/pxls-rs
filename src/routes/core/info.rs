use super::*;

#[derive(Serialize)]
pub struct ServerInfo {
	#[serde(skip_serializing_if = "Option::is_none")]
	name: Option<&'static str>,
	#[serde(skip_serializing_if = "Option::is_none")]
	version: Option<&'static str>,
	#[serde(skip_serializing_if = "Option::is_none")]
	source: Option<&'static str>,
	extensions: &'static [&'static str],
}

lazy_static! {
	static ref SERVER_INFO: ServerInfo = ServerInfo {
		name: Some("unnamed-newpxls-rs"),
		version: option_env!("CARGO_PKG_VERSION").filter(|s| !s.is_empty()),
		source: option_env!("CARGO_PKG_REPOSITORY").filter(|s| !s.is_empty()),
		extensions: &["authentication"],
	};
}

guard!(InfoAccess, Info);

#[get("/info")]
pub async fn get(user: Option<User>) -> HttpResponse {
	if user.unwrap_or_default().permissions.contains(&Permission::Info) {
		HttpResponse::Ok().json(&*SERVER_INFO)
	} else {
		actix_web::error::ErrorForbidden("Missing Permissions").into()
	}
}