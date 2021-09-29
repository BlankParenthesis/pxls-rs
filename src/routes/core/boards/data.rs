use super::*;

guard!(DataGetAccess, BoardsData);

// TODO: put intitial

#[get("/boards/{id}/data/colors")]
pub async fn get_colors(
	Path(id): Path<usize>,
	boards: BoardDataMap,
	range: RangeHeader,
	_access: DataGetAccess,
) -> Option<HttpResponse>  {
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
	_access: DataGetAccess,
) -> Option<HttpResponse>  {
	board!(boards[id]).map(|BoardData(board, _)| {
		range.respond_with(&board.read().unwrap().data.timestamps)
	})
}

#[get("/boards/{id}/data/mask")]
pub async fn get_mask(
	Path(id): Path<usize>,
	boards: BoardDataMap,
	range: RangeHeader,
	_access: DataGetAccess,
) -> Option<HttpResponse>  {
	board!(boards[id]).map(|BoardData(board, _)| {
		range.respond_with(&board.read().unwrap().data.mask)
	})
}

// TODO: put mask

#[get("/boards/{id}/data/initial")]
pub async fn get_initial(
	Path(id): Path<usize>,
	boards: BoardDataMap,
	range: RangeHeader,
	_access: DataGetAccess,
) -> Option<HttpResponse>  {
	board!(boards[id]).map(|BoardData(board, _)| {
		range.respond_with(&board.read().unwrap().data.initial)
	})
}