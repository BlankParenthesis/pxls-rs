use serde::Serialize;

#[derive(Serialize, Debug)]
pub struct Placement {
	#[serde(skip_serializing)]
	pub id: usize,
	pub position: usize,
	pub color: u8,
	pub timestamp: u32,
}