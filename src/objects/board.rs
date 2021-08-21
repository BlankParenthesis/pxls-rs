use serde::Serialize;

use crate::objects::color::Color;

#[derive(Serialize, Debug)]
pub struct BoardInfo {
	pub name: String,
	pub created_at: u64,
	pub shape: [[usize; 2]; 1], // TODO: support other shapes
	pub palette: Vec<Color>,
}

pub struct Board {
	pub meta: BoardInfo,
	color_data: Vec<u8>,
	mask_data: Vec<u8>,
	timestamp_data: Vec<u32>,
}

impl Board {
	pub fn new(
		name: String, 
		created_at: u64, 
		shape: [[usize; 2]; 1],
		palette: Vec<Color>,
	) -> Self {
		let [[width, height]] = shape;
		let size = width * height;

		Board {
			meta: BoardInfo {
				name,
				created_at,
				shape,
				palette,
			},
			color_data: vec![0; size],
			mask_data: vec![0; size],
			timestamp_data: vec![0; size],
		}
	}
}