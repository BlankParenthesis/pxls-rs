use std::sync::Arc;

use warp::{Reply, Rejection};
use warp::Filter;

use crate::filter::header::authorization::authorized;
use crate::filter::resource::board::{self, PassableBoard};
use crate::filter::resource::database;
use crate::permissions::Permission;
use crate::database::{BoardsDatabase, BoardsConnection, UsersDatabase};
use crate::BoardDataMap;

use serde::Serialize;

#[derive(Serialize, Debug)]
pub struct UserCount {
	pub active: usize,
	pub idle_timeout: u32,
}


pub fn get(
	boards: BoardDataMap,
	boards_db: Arc<BoardsDatabase>,
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("boards")
		.and(board::path::read(&boards))
		.and(warp::path("users"))
		.and(warp::path::end())
		.and(warp::get())
		.and(authorized(users_db, Permission::BoardsUsers.into()))
		.and(database::connection(boards_db))
		.then(|board: PassableBoard, _, _, connection: BoardsConnection| async move {
			let board = board.read().await;
			let board = board.as_ref().expect("Board went missing when getting user count");
			board.user_count(&connection).await
				.map(|active| {
					let idle_timeout = board.idle_timeout();
					warp::reply::json(&UserCount { active, idle_timeout })
				})
		})
}
