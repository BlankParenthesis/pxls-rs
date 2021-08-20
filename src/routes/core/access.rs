use actix_web::{get, HttpResponse};
use crate::access::permissions;

#[get("/access")]
pub async fn access() -> HttpResponse {
	HttpResponse::Ok()
		.json(&*permissions::DEFAULT_PERMISSIONS)
}