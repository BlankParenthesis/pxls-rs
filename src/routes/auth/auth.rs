use super::*;

#[derive(Serialize)]
pub struct AuthInfo {
	issuer: Url,
	#[serde(skip_serializing_if = "Option::is_none")]
	client_id: Option<String>,
}

lazy_static! {
	static ref AUTH_INFO: AuthInfo = {
		let config = crate::config::CONFIG.try_read().unwrap();

		AuthInfo {
			issuer: config.oidc_issuer.clone(),
			client_id: config.oidc_client_id.clone(),
		}
	};
}

guard!(InfoAccess, Info);

#[get("/auth")]
pub async fn get() -> HttpResponse {
	HttpResponse::Ok().json(&*AUTH_INFO)
}