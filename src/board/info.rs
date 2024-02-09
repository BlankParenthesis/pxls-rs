use serde::Serialize;

use super::{shape::Shape, color::Palette};

#[derive(Serialize, Debug)]
pub struct BoardInfo {
	pub name: String,
	pub created_at: u64,
	pub shape: Shape,
	pub palette: Palette,
	pub max_pixels_available: u32,
}
