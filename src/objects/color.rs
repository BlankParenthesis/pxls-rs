use serde::Serialize;

#[derive(Serialize, Debug)]
pub struct Color {
	pub name: String,
	pub value: u32,
}