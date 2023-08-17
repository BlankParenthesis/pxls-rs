use super::*;

use sea_orm::DatabaseConnection as Connection;

pub fn get(
	boards: BoardDataMap,
	database_pool: Arc<Connection>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("boards")
		.and(board::path::read(&boards))
		.and(warp::path("users"))
		.and(warp::path::end())
		.and(warp::get())
		.and(authorization::bearer().and_then(with_permission(Permission::BoardsUsers)))
		.and(database::connection(database_pool))
		.then(|board: PassableBoard, _user, connection: Arc<Connection>| async move {
			let board = board.read().await;
			let board = board.as_ref().expect("Board went missing when getting user count");
			match board.user_count(connection.as_ref()).await {
				Ok(user_count) => json(&user_count).into_response(),
				Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
			}
		})
}
