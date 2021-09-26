use actix_web::{get, HttpResponse};
use serde::Serialize;

#[derive(Serialize)]
pub struct AuthInfo {
	auth_uri: &'static str,
	token_uri: &'static str,
	client_id: Option<&'static str>,
}

const AUTH_INFO: AuthInfo = AuthInfo {
	auth_uri: "http://localhost:8180/auth/realms/pxls/protocol/openid-connect/auth",
	token_uri: "http://localhost:8180/auth/realms/pxls/protocol/openid-connect/token",
	client_id: Some("pxls"),
};

guard!(InfoAccess, Info);

#[get("/auth")]
pub async fn auth() -> HttpResponse {
	HttpResponse::Ok().json(AUTH_INFO)
}