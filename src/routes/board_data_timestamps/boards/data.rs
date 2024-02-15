use std::sync::Arc;

use warp::{
	reject::Rejection,
	reply::Reply,
	Filter,
};


use crate::filter::header::{
	authorization::{self, with_permission},
	range::{self, Range}
};
use crate::filter::resource::{board, database};
use crate::board::SectorBuffer;
use crate::BoardDataMap;
use crate::filter::resource::board::PassableBoard;
use crate::permissions::Permission;
use crate::database::{BoardsDatabase, BoardsConnection};

pub fn get_timestamps(
	boards: BoardDataMap,
	boards_db: Arc<BoardsDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("boards")
		.and(board::path::read(&boards))
		.and(warp::path("data"))
		.and(warp::path("timestamps"))
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
			let mut timestamp_data = board.as_ref()
				.expect("Board went missing when getting timestamp data")
				.read(SectorBuffer::Timestamps, &connection).await;
				
			range.respond_with(&mut timestamp_data).await
		})
}