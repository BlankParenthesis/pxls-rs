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
			let board = board.as_ref().unwrap();
			// TODO: bad unwrap?
			json(&board.user_count(connection.as_ref()).await.unwrap()).into_response()
		})
}
