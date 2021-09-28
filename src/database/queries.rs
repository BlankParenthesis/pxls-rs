use diesel::prelude::*;

use crate::objects::Board;
use crate::database::schema;

use super::Connection;

pub fn load_boards(connection: &Connection) -> QueryResult<Vec<Board>> {
	schema::board::table
		.load(connection)?
		.into_iter()
		.map(|board| Board::load(board, connection))
		.collect()
}