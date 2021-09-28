use serde::Deserialize;

#[derive(Deserialize, Debug)]
pub struct PlacementRequest {
	pub color: u8,
}