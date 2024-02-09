use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::database::boards::entities::color;

pub type Palette = HashMap<u32, Color>;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Color {
	pub name: String,
	pub value: u32,
}

impl From<color::Model> for Color {
	fn from(color: color::Model) -> Self {
		Color {
			name: color.name,
			value: color.value as u32,
		}
	}
}