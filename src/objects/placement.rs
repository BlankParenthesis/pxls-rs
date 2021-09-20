use serde::{Serialize, Deserialize};

#[derive(Serialize, Debug)]
pub struct Placement {
	#[serde(skip_serializing)]
	pub id: usize,
	pub position: usize,
	pub color: u8,
	pub timestamp: u32,
}

#[derive(Deserialize, Debug)]
pub struct PlacementRequest {
	pub color: u8,
}