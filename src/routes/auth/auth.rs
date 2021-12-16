use super::*;

#[derive(Serialize)]
pub struct AuthInfo {
	issuer: Url,
	#[serde(skip_serializing_if = "Option::is_none")]
	client_id: Option<String>,
}

lazy_static! {
	static ref AUTH_INFO: AuthInfo = {
		let config = crate::config::CONFIG.read().unwrap();

		AuthInfo {
			issuer: config.oidc_issuer.clone(),
			client_id: config.oidc_client_id.clone(),
		}
	};
}

pub fn get() -> impl Filter<Extract = impl Reply, Error = Rejection> + Clone {
	warp::path("auth")
		.and(warp::path::end())
		.and(warp::get())
		.map(|| warp::reply::with_status(json(&*AUTH_INFO), StatusCode::OK).into_response())
}
