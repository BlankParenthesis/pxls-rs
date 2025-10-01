use std::sync::Arc;

use warp::{Reply, Rejection};
use warp::Filter;

use crate::database::Database;
use crate::filter::header::authorization::authorized;
use crate::filter::resource::board::{self, PassableBoard};
use crate::permissions::Permission;
use crate::BoardDataMap;

use serde::Serialize;

#[derive(Serialize, Debug)]
pub struct UserCount {
	pub active: usize,
	pub idle_timeout: u32,
}

pub fn users(
	boards: BoardDataMap,
	db: Arc<Database>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("boards")
		.and(board::path::read(&boards))
		.and(warp::path("users"))
		.and(warp::path::end())
		.and(warp::get())
		.and(authorized(db, Permission::BoardsUsers.into()))
		.then(|board: PassableBoard, _, _| async move {
			let board = board.read().await;
			let board = board.as_ref().expect("Board went missing when getting user count");
			let active = board.user_count().await;
			let idle_timeout = board.idle_timeout();
			warp::reply::json(&UserCount { active, idle_timeout })
		})
}
