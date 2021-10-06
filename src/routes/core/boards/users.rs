use super::*;

guard!(BoardUsersAccess, BoardsUsers);

#[get("/boards/{id}/users")]
pub async fn get(
	Path(id): Path<usize>,
	boards: BoardDataMap,
	_access: BoardUsersAccess,
) -> Option<HttpResponse>  {
	if let Some(board) = board!(boards[id]) {
		let board = board.read().unwrap();
		let server = &board.server;
		let user_count = server.send(RequestUserCount {}).await.unwrap();
		
		Some(HttpResponse::Ok().json(user_count))
	} else {
		None
	}
}