use serde::{Serialize, Deserialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct Placement {
	pub position: u64,
	pub color: u8,
	pub timestamp: u32,
}