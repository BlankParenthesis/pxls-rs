use diesel::{Queryable, Insertable};
use serde::Serialize;

use super::schema::*;

#[derive(Queryable, Identifiable)]
#[table_name="board"]
pub struct Board {
	pub id: i32,
	pub name: String,
	pub created_at: i64,
	pub shape: String,
	pub mask: Vec<u8>,
	pub initial: Vec<u8>,
}

#[derive(Insertable)]
#[table_name="board"]
pub struct NewBoard {
	pub name: String,
	pub created_at: i64,
	pub shape: String,
	pub mask: Vec<u8>,
	pub initial: Vec<u8>,
}

#[derive(Queryable, Insertable, Identifiable, Associations)]
#[table_name="color"]
#[primary_key(board, index)]
#[belongs_to(Board, foreign_key = "board")]
pub struct Color {
	pub board: i32,
	pub index: i32,
	pub name: String,
	pub value: i32,
}

#[derive(Queryable, Identifiable, Associations, Serialize, Debug, Clone)]
#[table_name="placement"]
#[belongs_to(Board, foreign_key = "board")]
pub struct Placement {
	#[serde(skip_serializing)]
	pub id: i64,
	#[serde(skip_serializing)]
	pub board: i32,
	pub position: i64,
	pub color: i16,
	pub timestamp: i32,
	#[serde(skip_serializing)]
	pub user_id: Option<String>,
}

#[derive(Insertable)]
#[table_name="placement"]
pub struct NewPlacement {
	pub board: i32,
	pub position: i64,
	pub color: i16,
	pub timestamp: i32,
	pub user_id: Option<String>,
}