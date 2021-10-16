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
		// TODO: think of a good name. ideas:
		// iridescence / pearlescence

		// Using the pxls name seems bit presumptions given this shares
		// basically nothing with original pxls, but pxls-based names could be:
		// pxls-rs
		// pxls 2
		// neo-pxls

		name: Some("unnamed-newpxls-rs"),
		version: option_env!("CARGO_PKG_VERSION").filter(|s| !s.is_empty()),
		source: option_env!("CARGO_PKG_REPOSITORY").filter(|s| !s.is_empty()),
		extensions: &["authentication"],
	};
}

guard!(InfoAccess, Info);

#[get("/info")]
pub async fn get(user: AuthedUser) -> HttpResponse {
	let user = Option::<User>::from(user);
	if user.unwrap_or_default().permissions.contains(&Permission::Info) {
		HttpResponse::Ok().json(&*SERVER_INFO)
	} else {
		actix_web::error::ErrorForbidden("Missing Permissions").into()
	}
}