use std::sync::Arc;

use serde::Deserialize;
use tokio::sync::RwLock;

use warp::{
	http::{StatusCode, Uri, header},
	reject::Rejection,
	reply::{Reply, self, json},
	Filter,
};

use crate::{
	permissions::Permission,
	filter::{
		header::authorization::{self, with_permission},
		resource::{board::{self, PassableBoard, PendingDelete}, database},
		response::reference::Reference,
	},
	board::Palette,
	BoardDataMap,
	database::{BoardsDatabase, BoardsConnection},
};

#[derive(Deserialize, Debug)]
pub struct BoardInfoPost {
	pub name: String,
	pub shape: Vec<Vec<usize>>,
	pub palette: Palette,
	pub max_pixels_available: u32,
}

pub fn post(
	boards: BoardDataMap,
	boards_db: Arc<BoardsDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("boards")
		.and(warp::path::end())
		.and(warp::post())
		.and(warp::body::json())
		.and(authorization::bearer().and_then(with_permission(Permission::BoardsPost)))
		.and(database::connection(boards_db))
		.then(move |data: BoardInfoPost, _user, connection: BoardsConnection| {
			let boards = Arc::clone(&boards);
			async move {
				let board = connection.create_board(
					data.name,
					data.shape,
					data.palette,
					data.max_pixels_available,
				).await;

				let board = match board {
					Ok(board) => board,
					Err(err) => {
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

#[derive(Deserialize, Debug)]
pub struct BoardInfoPatch {
	pub name: Option<String>,
	pub shape: Option<Vec<Vec<usize>>>,
	pub palette: Option<Palette>,
	pub max_pixels_available: Option<u32>,
}

pub fn patch(
	boards: BoardDataMap,
	boards_db: Arc<BoardsDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("boards")
		.and(board::path::read(&boards))
		.and(warp::path::end())
		.and(warp::patch())
		// TODO: require application/merge-patch+json type?
		.and(warp::body::json())
		.and(authorization::bearer().and_then(with_permission(Permission::BoardsPatch)))
		.and(database::connection(boards_db))
		.then(|board: PassableBoard, patch: BoardInfoPatch, _user, connection: BoardsConnection| async move {
			let mut board = board.write().await;
			let board = board.as_mut().expect("Board went missing when patching");

			let update = board.update_info(
				patch.name,
				patch.shape,
				patch.palette,
				patch.max_pixels_available,
				&connection,
			);

			match update.await {
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
				Err(err) => {
					StatusCode::INTERNAL_SERVER_ERROR.into_response()
				},
			}
		})
}

pub fn delete(
	boards: BoardDataMap,
	boards_db: Arc<BoardsDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("boards")
		.and(board::path::prepare_delete(&boards))
		.and(warp::path::end())
		.and(warp::delete())
		.and(authorization::bearer().and_then(with_permission(Permission::BoardsDelete)))
		.and(database::connection(boards_db))
		.then(move |mut deletion: PendingDelete, _user, connection: BoardsConnection| async move {
			let board = deletion.perform();
			let mut board = board.write().await;
			let board = board.take().expect("Board went missing during deletion");
			match board.delete(&connection).await {
				Ok(()) => StatusCode::NO_CONTENT.into_response(),
				Err(err) => {
					StatusCode::INTERNAL_SERVER_ERROR.into_response()
				},
			}
		})
}