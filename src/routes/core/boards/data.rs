use std::sync::Arc;

use warp::{Filter, Reply, Rejection};

use crate::BoardDataMap;
use crate::filter::header::authorization::authorized;
use crate::filter::header::range::{self, Range};
use crate::filter::resource::board::{self, PassableBoard};
use crate::filter::resource::database;
use crate::permissions::Permission;
use crate::board::SectorBuffer;
use crate::database::{BoardsDatabase, BoardsConnection, UsersDatabase};

pub fn get_colors(
	boards: BoardDataMap,
	boards_db: Arc<BoardsDatabase>,
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("boards")
		.and(board::path::read(&boards))
		.and(warp::path("data"))
		.and(warp::path("colors"))
		.and(warp::path::end())
		.and(warp::get())
		.and(
			warp::any()
				.and(range::range())
				.or(range::default())
				.unify(),
		)
		.and(authorized(users_db, Permission::BoardsDataGet.into()))
		.and(database::connection(boards_db))
		.then(|board: PassableBoard, range: Range, _, _, connection: BoardsConnection| async move {
			// TODO: content disposition
			let board = board.read().await;
			let mut colors_data = board.as_ref()
				.expect("Board went missing when getting color data")
				.read(SectorBuffer::Colors, &connection).await;

			range.respond_with(&mut colors_data).await
		})
}