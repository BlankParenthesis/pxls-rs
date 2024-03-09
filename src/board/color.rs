use std::collections::HashMap;
use std::ops::Not;

use serde::{Deserialize, Serialize};

pub type Palette = HashMap<u32, Color>;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Color {
	pub name: String,
	pub value: u32,
	#[serde(default)]
	#[serde(skip_serializing_if = "<&bool>::not")]
	pub system_only: bool,
}
