use std::sync::Arc;
use sea_orm::DatabaseConnection as Connection;

use http::StatusCode;
use warp::{
	reject::Rejection,
	reply::{self, Reply},
	Filter,
};

use crate::filters::header::{authorization, range::{self, Range}};
use crate::filters::body::patch;
use crate::filters::resource::{board, database};
use crate::objects::*;
use crate::BoardDataMap;
use crate::filters::resource::board::PassableBoard;
use crate::filters::body::patch::BinaryPatch;
use crate::access::permissions::{with_permission, Permission};

pub fn get_mask(
	boards: BoardDataMap,
	database_pool: Arc<Connection>,
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
		.and(database::connection(database_pool))
		.then(|board: PassableBoard, range: Range, _user, connection: Arc<Connection>| async move {
			// TODO: content disposition
			let board = board.read().await;
			let mut mask_data = board.as_ref()
				.expect("Board went missing when getting mask data")
				.read(SectorBuffer::Mask, connection.as_ref()).await;

			range.respond_with(&mut mask_data).await
		})
}

pub fn patch_mask(
	boards: BoardDataMap,
	database_pool: Arc<Connection>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("boards")
		.and(board::path::read(&boards))
		.and(warp::path("data"))
		.and(warp::path("mask"))
		.and(warp::path::end())
		.and(warp::patch())
		.and(authorization::bearer().and_then(with_permission(Permission::BoardsDataPatch)))
		.and(patch::bytes())
		.and(database::connection(database_pool))
		.then(|board: PassableBoard, _user, patch: BinaryPatch, connection: Arc<Connection>| async move {
			// TODO: content disposition
			let board = board.write().await;
			let patch_result = board.as_ref()
				.expect("Board went missing when patching mask data")
				.try_patch_mask(&patch, connection.as_ref()).await;

			match patch_result {
				Ok(_) => StatusCode::NO_CONTENT.into_response(),
				Err(e) => reply::with_status(e, StatusCode::CONFLICT).into_response(),
			}
		})
}
