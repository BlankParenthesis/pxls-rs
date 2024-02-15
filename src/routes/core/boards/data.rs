use std::sync::Arc;

use warp::{Filter, Reply, Rejection};

use crate::{
	filter::{
		resource::{
			database,
			board::{PassableBoard, self},
		},
		header::{
			range::{Range, self},
			authorization::{self, with_permission},
		},
	},
	BoardDataMap,
	permissions::Permission,
	board::SectorBuffer,
	database::{BoardsDatabase, BoardsConnection},
};

pub fn get_colors(
	boards: BoardDataMap,
	boards_db: Arc<BoardsDatabase>,
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
		.and(authorization::bearer().and_then(with_permission(Permission::BoardsDataGet)))
		.and(database::connection(boards_db))
		.then(|board: PassableBoard, range: Range, _user, connection: BoardsConnection| async move {
			// TODO: content disposition
			let board = board.read().await;
			let mut colors_data = board.as_ref()
				.expect("Board went missing when getting color data")
				.read(SectorBuffer::Colors, &connection).await;

			range.respond_with(&mut colors_data).await
		})
}