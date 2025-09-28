use std::sync::Arc;

use reqwest::StatusCode;
use warp::{Reply, Rejection};
use warp::Filter;

use crate::filter::header::authorization::authorized;
use crate::filter::resource::board::{self, PassableBoard};
use crate::permissions::Permission;
use crate::BoardDataMap;
use crate::database::{BoardsConnection, BoardsDatabase, User};

pub fn delete(
	boards: BoardDataMap,
	boards_db: Arc<BoardsDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("boards")
		.and(board::path::read(&boards))
		.and(warp::path("pixels"))
		.and(warp::path::param())
		.and(warp::path::end())
		.and(warp::delete())
		.and(authorized(boards_db, Permission::BoardsPixelsUndo.into()))
		.then(|board: PassableBoard, position, user: Option<User>, connection: BoardsConnection| async move {
			let user = user.expect("Anonymous users cannot undo");

			let board = board.read().await;
			let board = board.as_ref().expect("Board went missing when creating a pixel");
			let undo_attempt = board.try_undo(&user, position, &connection).await;

			match undo_attempt {
				Ok(cooldown_info) => {
					let mut response = StatusCode::NO_CONTENT.into_response();

					for (key, value) in cooldown_info.into_headers() {
						response = warp::reply::with_header(response, key, value)
							.into_response();
					}

					response
				},
				Err(err) => err.into_response(),
			}
		})
}
