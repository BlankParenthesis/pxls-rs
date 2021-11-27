use actix::MessageResponse;
use serde::Serialize;

#[derive(Serialize, Debug, MessageResponse)]
pub struct UserCount {
	pub active: usize,
	pub idle_timeout: u32,
}
