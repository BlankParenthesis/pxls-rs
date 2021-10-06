use super::*;

guard!(BoardsPixelsListAccess, BoardsPixelsList);
guard!(BoardsPixelsGetAccess, BoardsPixelsGet);
guard!(BoardsPixelsPostAccess, BoardsPixelsPost);

#[get("/boards/{board_id}/pixels")]
pub async fn list(
	Path(board_id): Path<usize>,
	Query(options): Query<PaginationOptions<PageToken>>,
	boards: BoardDataMap,
	database_pool: Data<Pool>,
	_access: BoardsPixelsListAccess,
) -> Option<HttpResponse>  {
	board!(boards[board_id]).map(|BoardData(board, _)| {
		let page = options.page.unwrap_or_default();
		let limit = options.limit.unwrap_or(10).clamp(1, 100);

		let board = board.try_read().unwrap();
		let connection = &mut database_pool.get().unwrap();
		let previous_placements = board
			.list_placements(page.timestamp, page.id, limit, true, connection)
			.unwrap();
		let placements = board
			// Limit is +1 to get the start of the next page as the last element.
			// This is required for paging.
			.list_placements(page.timestamp, page.id, limit + 1, false, connection)
			.unwrap();
		
		fn page_uri(
			board_id:usize,
			timestamp: u32,
			placement_id: i64,
			limit: usize,
		) -> String {
			format!(
				"/boards/{}/pixels?page={}_{}&limit={}",
				board_id, timestamp, placement_id, limit
			)
		}

		HttpResponse::Ok()
			.json(Page {
				previous: previous_placements.get(0)
					.map(|placement| page_uri(
						board_id,
						placement.timestamp as u32,
						placement.id,
						limit,
					)),
				items: &placements[..placements.len().clamp(0, limit)],
				next: (placements.len() > limit)
					.then(|| placements.iter().last().unwrap())
					.map(|placement| page_uri(
						board_id,
						placement.timestamp as u32,
						placement.id,
						limit,
					)),
			})
	})
}

#[get("/boards/{id}/pixels/{position}")]
pub async fn get(
	Path((id, position)): Path<(usize, u64)>,
	boards: BoardDataMap,
	database_pool: Data<Pool>,
	_access: BoardsPixelsGetAccess,
) -> Option<HttpResponse> {
	board!(boards[id]).and_then(|BoardData(board, _)| {
		let board = board.try_read().unwrap();
		let connection = &mut database_pool.get().unwrap();

		board.lookup(position, connection).unwrap()
			.map(|placement| HttpResponse::Ok().json(placement))
	})
}

#[post("/boards/{id}/pixels/{position}")]
pub async fn post(
	Path((id, position)): Path<(usize, u64)>,
	Json(placement): Json<PlacementRequest>,
	boards: BoardDataMap,
	database_pool: Data<Pool>,
	user: User,
	_access: BoardsPixelsPostAccess,
) -> Option<HttpResponse> {
	board!(boards[id]).and_then(|BoardData(board, server)| {
		let mut board = board.try_write().unwrap();
		let connection = &mut database_pool.get().unwrap();
		
		Some(match board.try_place(&user, position, placement.color, connection) {
			Ok(placement) => {
				server.do_send(Place { placement: placement.clone() });
				HttpResponse::build(StatusCode::CREATED)
					.json(placement)
			},
			Err(e) => actix_web::Error::from(e).into(),
		})
	})
}

