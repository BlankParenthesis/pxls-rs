use std::sync::Arc;
use sea_orm::DatabaseConnection as Connection;

use warp::{
	reject::Rejection,
	reply::Reply,
	Filter,
};

use crate::filters::header::{authorization, range::{self, Range}};
use crate::filters::resource::{board, database};
use crate::board::sector::*;
use crate::BoardDataMap;
use crate::filters::resource::board::PassableBoard;
use crate::permissions::{with_permission, Permission};

pub fn get_timestamps(
	boards: BoardDataMap,
	database_pool: Arc<Connection>,
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
		.and(database::connection(database_pool))
		.then(|board: PassableBoard, range: Range, _user, connection: Arc<Connection>| async move {
			// TODO: content disposition
			let board = board.read().await;
			let mut timestamp_data = board.as_ref()
				.expect("Board went missing when getting timestamp data")
				.read(SectorBuffer::Timestamps, connection.as_ref()).await;
				
			range.respond_with(&mut timestamp_data).await
		})
}