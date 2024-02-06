use std::sync::Arc;

use sea_orm::DatabaseConnection as Connection;
use warp::{Filter, Reply, Rejection};

use crate::{
	filters::{
		resource::{
			database,
			board::{PassableBoard, self},
		},
		header::{
			range::{Range, self},
			authorization,
		},
	},
	BoardDataMap,
	permissions::{with_permission, Permission},
	board::sector::buffer::SectorBuffer,
};

pub fn get_colors(
	boards: BoardDataMap,
	database_pool: Arc<Connection>,
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
		.and(database::connection(database_pool))
		.then(|board: PassableBoard, range: Range, _user, connection: Arc<Connection>| async move {
			// TODO: content disposition
			let board = board.read().await;
			let mut colors_data = board.as_ref()
				.expect("Board went missing when getting color data")
				.read(SectorBuffer::Colors, connection.as_ref()).await;

			range.respond_with(&mut colors_data).await
		})
}