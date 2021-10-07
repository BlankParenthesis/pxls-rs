use serde::Serialize;
use actix::MessageResponse;

#[derive(Serialize, Debug, MessageResponse)]
pub struct UserCount {
	pub active: usize,
	pub idle_timeout: u32,
}
