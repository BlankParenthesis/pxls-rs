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

pub fn get() -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("info")
		.and(warp::path::end())
		.and(warp::get())
		.and(authorization::bearer().and_then(with_permission(Permission::Info)))
		.map(|_user| warp::reply::with_status(json(&*SERVER_INFO), StatusCode::OK).into_response())
}
