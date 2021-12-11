use serde::Serialize;

#[derive(Serialize, Debug)]
pub struct UserCount {
	pub active: usize,
	pub idle_timeout: u32,
}
