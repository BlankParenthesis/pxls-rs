use std::sync::Arc;

use reqwest::StatusCode;
use warp::reply::json;
use warp::{Reply, Rejection};
use warp::Filter;

use crate::filter::resource::{board, database};
use crate::{
	permissions::Permission,
	filter::header::authorization::{self, with_permission},
	filter::resource::board::PassableBoard,
	BoardDataMap,
};
use crate::database::{BoardsDatabase, BoardsConnection};

use serde::Serialize;

#[derive(Serialize, Debug)]
pub struct UserCount {
	pub active: usize,
	pub idle_timeout: u32,
}


pub fn get(
	boards: BoardDataMap,
	boards_db: Arc<BoardsDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("boards")
		.and(board::path::read(&boards))
		.and(warp::path("users"))
		.and(warp::path::end())
		.and(warp::get())
		.and(authorization::bearer().and_then(with_permission(Permission::BoardsUsers)))
		.and(database::connection(boards_db))
		.then(|board: PassableBoard, _user, connection: BoardsConnection| async move {
			let board = board.read().await;
			let board = board.as_ref().expect("Board went missing when getting user count");
			match board.user_count(&connection).await {
				Ok(active) => {
					let idle_timeout = board.idle_timeout();
					json(&UserCount { active, idle_timeout }).into_response()
				},
				Err(err) => {
					StatusCode::INTERNAL_SERVER_ERROR.into_response()
				},
			}
		})
}
