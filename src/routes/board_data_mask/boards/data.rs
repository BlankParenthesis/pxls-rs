use std::sync::Arc;

use warp::{
	http::StatusCode,
	reject::Rejection,
	reply::{self, Reply},
	Filter,
};


use crate::filter::header::{
	authorization::{self, with_permission},
	range::{self, Range}
};
use crate::filter::body::patch;
use crate::filter::resource::{board, database};
use crate::board::SectorBuffer;
use crate::BoardDataMap;
use crate::filter::resource::board::PassableBoard;
use crate::filter::body::patch::BinaryPatch;
use crate::permissions::Permission;
use crate::database::{BoardsConnection, BoardsDatabase};

pub fn get_mask(
	boards: BoardDataMap,
	boards_db: Arc<BoardsDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("boards")
		.and(board::path::read(&boards))
		.and(warp::path("data"))
		.and(warp::path("mask"))
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
			let mut mask_data = board.as_ref()
				.expect("Board went missing when getting mask data")
				.read(SectorBuffer::Mask, &connection).await;

			range.respond_with(&mut mask_data).await
		})
}

pub fn patch_mask(
	boards: BoardDataMap,
	boards_db: Arc<BoardsDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("boards")
		.and(board::path::read(&boards))
		.and(warp::path("data"))
		.and(warp::path("mask"))
		.and(warp::path::end())
		.and(warp::patch())
		.and(authorization::bearer().and_then(with_permission(Permission::BoardsDataPatch)))
		.and(patch::bytes())
		.and(database::connection(boards_db))
		.then(|board: PassableBoard, _user, patch: BinaryPatch, connection: BoardsConnection| async move {
			// TODO: content disposition
			let board = board.write().await;
			let patch_result = board.as_ref()
				.expect("Board went missing when patching mask data")
				.try_patch_mask(&patch, &connection).await;

			match patch_result {
				Ok(_) => StatusCode::NO_CONTENT.into_response(),
				Err(e) => reply::with_status(e, StatusCode::CONFLICT).into_response(),
			}
		})
}
