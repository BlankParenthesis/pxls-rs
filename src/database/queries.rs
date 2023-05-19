use diesel::prelude::*;

use super::Connection;
use crate::{database::schema, objects::Board};

pub fn load_boards(connection: &mut Connection) -> QueryResult<Vec<Board>> {
	schema::board::table
		.load(connection)?
		.into_iter()
		.map(|board| Board::load(board, connection))
		.collect()
}
