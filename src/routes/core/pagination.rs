use serde::Serialize;

#[derive(Serialize)]
pub struct Page<'t, T> {
	pub items: &'t [T],
	pub next: Option<String>,
	pub previous: Option<String>,
}
