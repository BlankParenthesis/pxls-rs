use std::sync::Arc;

use reqwest::StatusCode;
use warp::{Reply, Rejection};
use warp::Filter;

use crate::filter::header::authorization::authorized;
use crate::permissions::Permission;
use crate::BoardDataMap;
use crate::database::{Database, DbConn, PlacementSpecifier, Specifier, User};

pub fn delete(
	boards: BoardDataMap,
	boards_db: Arc<Database>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	PlacementSpecifier::path()
		.and(warp::delete())
		.and(authorized(boards_db, Permission::BoardsPixelsUndo.into()))
		.then(move |placement: PlacementSpecifier, user: Option<User>, connection: DbConn| {
			let boards = boards.clone();
			async move {
				let user = user.expect("Anonymous users cannot undo");
	
				let boards = boards.read().await;
				let board = boards.get(&placement.board())
					.ok_or(StatusCode::NOT_FOUND)?;
				let mut board = board.write().await;
				let board = board.as_mut()
					.expect("Board went missing when creating a pixel");
				
				let cooldown_info = board.try_undo(&user, &placement, &connection).await?;
	
				let mut response = StatusCode::NO_CONTENT.into_response();
	
				for (key, value) in cooldown_info.into_headers() {
					response = warp::reply::with_header(response, key, value)
						.into_response();
				}
	
				Ok::<_, StatusCode>(response)
			}
		})
}
