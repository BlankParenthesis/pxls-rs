use super::*;

guard!(BoardUsersAccess, BoardsUsers);

#[get("/boards/{id}/users")]
pub async fn get(
	Path(id): Path<usize>,
	boards: BoardDataMap,
	database_pool: Data<Pool>,
	_access: BoardUsersAccess,
) -> Option<HttpResponse>  {
	if let Some(board) = board!(boards[id]) {
		let board = board.read().unwrap();
		let connection = database_pool.get().unwrap();
		let user_count = board.user_count(&connection).unwrap();
		
		Some(HttpResponse::Ok().json(user_count))
	} else {
		None
	}
}