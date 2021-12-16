use super::*;

pub fn get(
	boards: BoardDataMap,
	database_pool: Arc<Pool>,
) -> impl Filter<Extract = impl Reply, Error = Rejection> + Clone {
	warp::path("boards")
		.and(board::path::read(&boards))
		.and(warp::path("users"))
		.and(warp::path::end())
		.and(warp::get())
		.and(authorization::bearer().and_then(with_permission(Permission::BoardsUsers)))
		.and(database::connection(database_pool))
		.map(|board: PassableBoard, _user, connection| {
			let board = board.read();
			let board = board.as_ref().unwrap();
			json(&board.user_count(&connection).unwrap()).into_response()
		})
}
