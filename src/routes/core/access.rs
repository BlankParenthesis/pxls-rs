use rocket::serde::json::{json, Value};
use crate::access::permissions;

#[get("/access")]
pub fn access() -> Value {
	json!(&*permissions::DEFAULT_PERMISSIONS)
}