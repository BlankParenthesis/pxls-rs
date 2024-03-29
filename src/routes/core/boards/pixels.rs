use super::*;

pub fn list(
	boards: BoardDataMap,
	database_pool: Arc<Pool>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("boards")
		.and(board::path::read(&boards))
		.and(warp::path("pixels"))
		.and(warp::path::end())
		.and(warp::get())
		.and(authorization::bearer().and_then(with_permission(Permission::BoardsPixelsList)))
		.and(warp::query())
		.and(database::connection(Arc::clone(&database_pool)))
		.map(|board: PassableBoard, _user, options: PaginationOptions<PageToken>, mut connection| {
			let page = options.page.unwrap_or_default();
			let limit = options
				.limit
				.unwrap_or(10)
				.clamp(1, 100);

			let board = board.read();
			let board = board.as_ref().unwrap();
			let previous_placements = board
				.list_placements(page.timestamp, page.id, limit, true, &mut connection)
				.unwrap();
			let placements = board
			// Limit is +1 to get the start of the next page as the last element.
			// This is required for paging.
			.list_placements(page.timestamp, page.id, limit + 1, false, &mut connection)
			.unwrap();

			fn page_uri(
				board_id: i32,
				timestamp: u32,
				placement_id: i64,
				limit: usize,
			) -> String {
				format!(
					"/boards/{}/pixels?page={}_{}&limit={}",
					board_id, timestamp, placement_id, limit
				)
			}

			json(&Page {
				previous: previous_placements
					.get(0)
					.map(|placement| {
						page_uri(board.id, placement.timestamp as u32, placement.id, limit)
					}),
				items: &placements[..placements.len().clamp(0, limit)],
				next: (placements.len() > limit)
					.then(|| placements.iter().last().unwrap())
					.map(|placement| {
						page_uri(board.id, placement.timestamp as u32, placement.id, limit)
					}),
			})
			.into_response()
		})
}

pub fn get(
	boards: BoardDataMap,
	database_pool: Arc<Pool>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("boards")
		.and(board::path::read(&boards))
		.and(warp::path("pixels"))
		.and(warp::path::param())
		.and(warp::path::end())
		.and(warp::get())
		.and(authorization::bearer().and_then(with_permission(Permission::BoardsPixelsGet)))
		.and(database::connection(Arc::clone(&database_pool)))
		.map(|board: PassableBoard, position, _user, mut connection| {
			let board = board.read();
			let board = board.as_ref().unwrap();
			let placement = board
				.lookup(position, &mut connection)
				.unwrap();

			placement
				.map(|placement| json(&placement).into_response())
				.unwrap_or_else(|| StatusCode::NOT_FOUND.into_response())
		})
}

pub fn post(
	boards: BoardDataMap,
	database_pool: Arc<Pool>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("boards")
		.and(board::path::read(&boards))
		.and(warp::path("pixels"))
		.and(warp::path::param())
		.and(warp::path::end())
		.and(warp::post())
		.and(warp::body::json())
		.and(authorization::bearer().and_then(with_permission(Permission::BoardsPixelsPost)))
		.and(database::connection(Arc::clone(&database_pool)))
		.map(|board: PassableBoard, position, placement: PlacementRequest, user, mut connection| {
			let user =
				Option::from(user).expect("Default user shouldn't have place permisisons");

			let board = board.write();
			let board = board.as_ref().unwrap();
			let place_attempt = board.try_place(
				// TODO: maybe accept option but make sure not to allow undos etc for anon
				// users
				&user,
				position,
				placement.color,
				&mut connection,
			);

			match place_attempt {
				Ok(placement) => {
					let cooldown_info = board
						.user_cooldown_info(&user, &mut connection)
						.unwrap();

					let mut response = warp::reply::with_status(
						json(&placement).into_response(),
						StatusCode::CREATED,
					)
					.into_response();

					for (key, value) in cooldown_info.into_headers() {
						response =
							warp::reply::with_header(response, key, value).into_response();
					}

					response
				},
				Err(err) => err.into_response(),
			}
		})
}
