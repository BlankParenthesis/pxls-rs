use std::sync::Arc;

use tokio::sync::RwLock;
use sea_orm::DatabaseConnection as Connection;

use warp::{
	http::{StatusCode, Uri, header},
	reject::Rejection,
	reply::{Reply, self, json},
	Filter,
};

use crate::{
	permissions::Permission,
	filter::{
		header::authorization::{self, with_permission, Bearer},
		resource::{board::{self, PassableBoard, PendingDelete}, database},
		response::reference::Reference,
	},
	board::Board,
	board::{BoardInfoPost, BoardInfoPatch},
	BoardDataMap,
};

pub fn post(
	boards: BoardDataMap,
	database_pool: Arc<Connection>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("boards")
		.and(warp::path::end())
		.and(warp::post())
		.and(warp::body::json())
		.and(authorization::bearer().and_then(with_permission(Permission::BoardsPost)))
		.and(database::connection(database_pool))
		.then(move |data: BoardInfoPost, _user, connection: Arc<Connection>| {
			let boards = Arc::clone(&boards);
			async move {
				let board = Board::create(data, connection.as_ref()).await;

				let board = match board {
					Ok(board) => board,
					Err(_) => {
						return StatusCode::INTERNAL_SERVER_ERROR.into_response();
					}
				};

				let id = board.id as usize;

				let mut boards = boards.write().await;
				boards.insert(id, Arc::new(RwLock::new(Some(board))));

				let board = boards.get(&id).expect("Board went missing from list during creation")
					.read().await;
				let board = board.as_ref().expect("Board went missing during creation");

				let mut response = json(&Reference::from(board)).into_response();
				response = reply::with_status(response, StatusCode::CREATED).into_response();
				response = reply::with_header(
					response,
					header::LOCATION,
					Uri::from(board).to_string(),
				)
				.into_response();
				response
			}
		})
}

pub fn patch(
	boards: BoardDataMap,
	database_pool: Arc<Connection>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("boards")
		.and(board::path::read(&boards))
		.and(warp::path::end())
		.and(warp::patch())
		// TODO: require application/merge-patch+json type?
		.and(warp::body::json())
		.and(authorization::bearer().and_then(with_permission(Permission::BoardsPatch)))
		.and(database::connection(database_pool))
		.then(|board: PassableBoard, patch: BoardInfoPatch, _user, connection: Arc<Connection>| async move {
			let mut board = board.write().await;
			let board = board.as_mut().expect("Board went missing when patching");

			match board.update_info(patch, connection.as_ref()).await {
				Ok(()) => {
					let mut response = json(&Reference::from(&*board)).into_response();
					response = reply::with_status(
						response,
						StatusCode::CREATED,
					).into_response();
					response = reply::with_header(
						response,
						header::LOCATION,
						Uri::from(&*board).to_string(),
					).into_response();
					response
				},
				Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
			}
		})
}

pub fn delete(
	boards: BoardDataMap,
	database_pool: Arc<Connection>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("boards")
		.and(board::path::prepare_delete(&boards))
		.and(warp::path::end())
		.and(warp::delete())
		.and(authorization::bearer().and_then(with_permission(Permission::BoardsDelete)))
		.and(database::connection(database_pool))
		.then(move |mut deletion: PendingDelete, _user, connection: Arc<Connection>| async move {
			let board = deletion.perform();
			let mut board = board.write().await;
			let board = board.take().expect("Board went missing during deletion");
			match board.delete(connection.as_ref()).await {
				Ok(()) => StatusCode::NO_CONTENT.into_response(),
				Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
			}
		})
}