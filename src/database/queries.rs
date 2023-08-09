use sea_orm::{ConnectionTrait, EntityTrait};

use crate::objects::Board;

use super::{entities::board, DbResult};

pub async fn load_boards<Connection: ConnectionTrait>(
	connection: &Connection
) -> DbResult<Vec<Board>> {
	let db_boards = board::Entity::find()
		.all(connection).await?;

	let mut boards = Vec::with_capacity(db_boards.len());

	for board in db_boards {
		boards.push(Board::load(board, connection).await?);
	}
	
	Ok(boards)
}		
