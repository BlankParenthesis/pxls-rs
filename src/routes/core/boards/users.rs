use super::*;

guard!(BoardUsersAccess, BoardsUsers);

#[get("/boards/{id}/users")]
pub async fn get(
	Path(id): Path<usize>,
	boards: BoardDataMap,
	_access: BoardUsersAccess,
) -> Option<HttpResponse>  {
	if let Some(BoardData(_, server)) = board!(boards[id]) {
		let user_count = server.send(RequestUserCount {}).await.unwrap();
		
		Some(HttpResponse::Ok().json(user_count))
	} else {
		None
	}
}