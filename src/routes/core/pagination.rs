use serde::{Serialize, Deserialize};

#[derive(Serialize)]
pub struct Page<'t, T> {
	pub items: &'t [T],
	pub next: Option<String>,
	pub previous: Option<String>,
}

#[derive(Deserialize)]
pub struct PaginationOptions {
	pub page: Option<usize>,
	pub limit: Option<usize>,
}
