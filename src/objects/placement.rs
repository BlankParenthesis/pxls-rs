use serde::Serialize;
use crate::objects::color::Color;

#[derive(Serialize, Debug)]
pub struct Placement {
	pub position: [u64; 2], // TODO: support more/less axis
	pub color: Color,
	pub modified: u64,
}