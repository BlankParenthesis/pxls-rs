use serde::Serialize;

#[derive(Serialize, Debug)]
pub struct Placement {
	pub position: usize,
	pub color: u8,
	pub modified: u32,
}