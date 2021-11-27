use super::*;

#[get("/access")]
pub async fn get(user: AuthedUser) -> HttpResponse {
	let permissions = Option::<User>::from(user)
		.unwrap_or_default()
		.permissions;

	HttpResponse::Ok().json(permissions)
}
