use super::*;

#[get("/access")]
pub async fn get(user: Option<User>) -> HttpResponse {
	let permissions = user.unwrap_or_default().permissions;

	HttpResponse::Ok()
		.json(permissions)
}