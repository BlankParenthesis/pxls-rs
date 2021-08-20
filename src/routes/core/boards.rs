use rocket::response::Redirect;
use rocket::serde::json::{json, Value};
use serde::Serialize;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::access::permissions;
use crate::routes::core::pagination::Page;

guard!(BoardListAccess, BoardsList);
guard!(BoardGetAccess, BoardsGet);
guard!(SocketAccesss, SocketCore);

#[derive(Serialize)]
pub struct Color {
	name: String,
	value: u32,
}

#[derive(Serialize)]
pub struct BoardInfo {
	name: String,
	created_at: u64,
	shape: [[u64; 2]; 1],
	palette: Vec<Color>,
}

lazy_static! {
	pub static ref BOARD_INFO: BoardInfo = BoardInfo {
		name: String::from("c0"),
		created_at: SystemTime::now()
			.duration_since(UNIX_EPOCH)
			.unwrap()
			.as_millis() as u64,
		shape: [[1000, 1000]],
		palette: vec![Color { name: String::from("red"), value: 0xff0000 }],
	};
}

#[get("/boards?<page>&<limit>")]
pub fn list(page: Option<usize>, limit: Option<usize>, _access: BoardListAccess) -> Value {
	let page = page.unwrap_or(0);
	let limit = limit.unwrap_or(2).clamp(0, 10);
	
	let items = vec![&*BOARD_INFO];
	let mut chunks = items.chunks(limit);
	
	fn page_uri(page: usize, limit: usize) -> String {
		format!("/boards?page={}&limit={}", page, limit)
	}

	json!(
		Page {
			previous: page.checked_sub(1).and_then(
				|page| chunks
					.nth(page)
					.map(|_| page_uri(page, limit)),
			),
			items: chunks
				.next()
				.unwrap_or(&[]),
			next: page.checked_add(1).and_then(
				|page| chunks
					.next()
					.map(|_| page_uri(page, limit)),
			),
		}
	)
}

#[get("/boards/default", rank = 0)]
pub fn get_default(_access: BoardGetAccess) -> Option<Redirect>  {
	Some(Redirect::temporary("/boards/0"))
}

#[get("/boards/<id>", rank = 1)]
pub fn get(id: u32, _access: BoardGetAccess) -> Option<Value>  {
	if id == 0 {
		Some(json!(&*BOARD_INFO))
	} else {
		None
	}
}
