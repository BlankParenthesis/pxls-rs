use serde::{Deserialize, Serialize};

use crate::socket::packet;

use super::{shape::Shape, color::Palette};

#[derive(Serialize, Debug)]
pub struct BoardInfo {
	pub name: String,
	pub created_at: u64,
	pub shape: Shape,
	pub palette: Palette,
	pub max_pixels_available: u32,
}

#[derive(Deserialize, Debug)]
pub struct BoardInfoPost {
	pub name: String,
	pub shape: Shape,
	pub palette: Palette,
	pub max_pixels_available: u32,
}

#[derive(Deserialize, Debug)]
pub struct BoardInfoPatch {
	pub name: Option<String>,
	pub shape: Option<Shape>,
	pub palette: Option<Palette>,
	pub max_pixels_available: Option<u32>,
}

impl From<BoardInfoPatch> for packet::server::BoardInfo {
	fn from(info: BoardInfoPatch) -> Self {
		Self {
			name: info.name,
			shape: info.shape,
			palette: info.palette,
			max_pixels_available: info.max_pixels_available,
		}
	}
}