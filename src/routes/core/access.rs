use actix_web::{get, HttpResponse};

use crate::objects::User;

#[get("/access")]
pub async fn access(user: Option<User>) -> HttpResponse {
	let permissions = user.unwrap_or_default().permissions;

	HttpResponse::Ok()
		.json(permissions)
}