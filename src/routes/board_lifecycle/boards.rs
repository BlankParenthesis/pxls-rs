use std::sync::Arc;

use serde::Deserialize;
use tokio::sync::RwLock;

use warp::{
	http::StatusCode,
	reject::Rejection,
	reply::Reply,
	Filter,
};

use crate::BoardDataMap;
use crate::board::Palette;
use crate::filter::header::authorization::authorized;
use crate::filter::resource::board::{self, PassableBoard, PendingDelete};
use crate::filter::resource::database;
use crate::filter::response::reference;
use crate::permissions::Permission;
use crate::database::{BoardsDatabase, BoardsConnection, UsersDatabase};

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
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("boards")
		.and(warp::path::end())
		.and(warp::post())
		.and(warp::body::json())
		.and(authorized(users_db, Permission::BoardsPost.into()))
		.and(database::connection(boards_db))
		.then(move |data: BoardInfoPost, _, _, connection: BoardsConnection| {
			let boards = Arc::clone(&boards);
			async move {
				let board = connection.create_board(
					data.name,
					data.shape,
					data.palette,
					data.max_pixels_available,
				).await?;

				let id = board.id as usize;

				let mut boards = boards.write().await;
				boards.insert(id, Arc::new(RwLock::new(Some(board))));

				let board = boards.get(&id).expect("Board went missing from list during creation")
					.read().await;
				let board = board.as_ref().expect("Board went missing during creation");

				Ok::<_, StatusCode>(reference::created(board))
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
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("boards")
		.and(board::path::read(&boards))
		.and(warp::path::end())
		.and(warp::patch())
		// TODO: require application/merge-patch+json type?
		.and(warp::body::json())
		.and(authorized(users_db, Permission::BoardsPatch.into()))
		.and(database::connection(boards_db))
		.then(|board: PassableBoard, patch: BoardInfoPatch, _, _, connection: BoardsConnection| async move {
			let mut board = board.write().await;
			let board = board.as_mut().expect("Board went missing when patching");

			board.update_info(
				patch.name,
				patch.shape,
				patch.palette,
				patch.max_pixels_available,
				&connection,
			).await
				.map(|()| reference::created(board)) // TODO: is "created" correct?
		})
}

pub fn delete(
	boards: BoardDataMap,
	boards_db: Arc<BoardsDatabase>,
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("boards")
		.and(board::path::prepare_delete(&boards))
		.and(warp::path::end())
		.and(warp::delete())
		.and(authorized(users_db, Permission::BoardsDelete.into()))
		.and(database::connection(boards_db))
		.then(move |mut deletion: PendingDelete, _, _, connection: BoardsConnection| async move {
			let board = deletion.perform();
			let mut board = board.write().await;
			board.take()
				.expect("Board went missing during deletion")
				.delete(&connection).await
				.map(|_| StatusCode::NO_CONTENT)
		})
}