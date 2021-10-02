use super::*;

guard!(BoardsDataGetAccess, BoardsDataGet);
guard!(BoardsDataPatchAccess, BoardsDataPatch);

#[get("/boards/{id}/data/colors")]
pub async fn get_colors(
	Path(id): Path<usize>,
	boards: BoardDataMap,
	range: RangeHeader,
	_access: BoardsDataGetAccess,
) -> Option<HttpResponse> {
	board!(boards[id]).map(|BoardData(board, _)| {
		// TODO: content disposition
		range.respond_with(&board.read().unwrap().data.colors)
	})
}

#[get("/boards/{id}/data/timestamps")]
pub async fn get_timestamps(
	Path(id): Path<usize>,
	boards: BoardDataMap,
	range: RangeHeader,
	_access: BoardsDataGetAccess,
) -> Option<HttpResponse> {
	board!(boards[id]).map(|BoardData(board, _)| {
		range.respond_with(&board.read().unwrap().data.timestamps)
	})
}

#[get("/boards/{id}/data/mask")]
pub async fn get_mask(
	Path(id): Path<usize>,
	boards: BoardDataMap,
	range: RangeHeader,
	_access: BoardsDataGetAccess,
) -> Option<HttpResponse> {
	board!(boards[id]).map(|BoardData(board, _)| {
		range.respond_with(&board.read().unwrap().data.mask)
	})
}

#[get("/boards/{id}/data/initial")]
pub async fn get_initial(
	Path(id): Path<usize>,
	boards: BoardDataMap,
	range: RangeHeader,
	_access: BoardsDataGetAccess,
) -> Option<HttpResponse> {
	board!(boards[id]).map(|BoardData(board, _)| {
		range.respond_with(&board.read().unwrap().data.initial)
	})
}

#[patch("/boards/{id}/data/initial")]
pub async fn patch_initial(
	Path(id): Path<usize>,
	patch_info: BinaryPatch,
	database_pool: Data<Pool>,
	boards: BoardDataMap,
	_access: BoardsDataPatchAccess,
) -> Option<HttpResponse> {
	board!(boards[id]).map(|BoardData(board, _)| {
		let mut board = board.write().unwrap();

		board.try_patch_initial(&patch_info, &database_pool.get().unwrap())
			.map(|_| HttpResponse::NoContent().finish())
			.unwrap_or_else(|e| error::ErrorConflict(e).into())
	})
}

#[patch("/boards/{id}/data/mask")]
pub async fn patch_mask(
	Path(id): Path<usize>,
	patch_info: BinaryPatch,
	database_pool: Data<Pool>,
	boards: BoardDataMap,
	_access: BoardsDataPatchAccess,
) -> Option<HttpResponse> {
	board!(boards[id]).map(|BoardData(board, _)| {
		let mut board = board.write().unwrap();

		board.try_patch_mask(&patch_info, &database_pool.get().unwrap())
			.map(|_| HttpResponse::NoContent().finish())
			.unwrap_or_else(|e| error::ErrorConflict(e).into())
	})
}