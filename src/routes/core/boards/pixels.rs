use super::*;

use sea_orm::DatabaseConnection as Connection;

pub fn list(
	boards: BoardDataMap,
	database_pool: Arc<Connection>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("boards")
		.and(board::path::read(&boards))
		.and(warp::path("pixels"))
		.and(warp::path::end())
		.and(warp::get())
		.and(authorization::bearer().and_then(with_permission(Permission::BoardsPixelsList)))
		.and(warp::query())
		.and(database::connection(Arc::clone(&database_pool)))
		.then(|board: PassableBoard, _user, options: PaginationOptions<PageToken>, connection: Arc<Connection>| async move {
			let page = options.page.unwrap_or_default();
			let limit = options
				.limit
				.unwrap_or(10)
				.clamp(1, 100);

			let board = board.read().await;
			let board = board.as_ref().expect("Board went missing when listing pixels");
			let previous_placements = board.list_placements(
				page.timestamp,
				page.id,
				limit,
				true,
				connection.as_ref(),
			).await;

			let previous_placements = match previous_placements {
				Ok(placements) => placements,
				Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
			};

			let placements = board.list_placements(
				page.timestamp,
				page.id,
				// Limit is +1 to get the start of the next page as the last element.
				// This is required for paging.
				limit + 1, false, connection.as_ref()
			).await;

			let placements = match placements {
				Ok(placements) => placements,
				Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
			};

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
					.last()
					.map(|placement| {
						page_uri(board.id, placement.timestamp as u32, placement.id, previous_placements.len())
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
	database_pool: Arc<Connection>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("boards")
		.and(board::path::read(&boards))
		.and(warp::path("pixels"))
		.and(warp::path::param())
		.and(warp::path::end())
		.and(warp::get())
		.and(authorization::bearer().and_then(with_permission(Permission::BoardsPixelsGet)))
		.and(database::connection(Arc::clone(&database_pool)))
		.then(|board: PassableBoard, position, _user, connection: Arc<Connection>| async move {
			let board = board.read().await;
			let board = board.as_ref().expect("Board went missing when getting a pixel");
			match board.lookup(position, connection.as_ref()).await {
				Ok(placement) => {
					placement
						.map(|placement| json(&placement).into_response())
						.unwrap_or_else(|| StatusCode::NOT_FOUND.into_response())
				},
				Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
			}
		})
}

pub fn post(
	boards: BoardDataMap,
	database_pool: Arc<Connection>,
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
		.then(|board: PassableBoard, position, placement: PlacementRequest, user, connection: Arc<Connection>| async move {
			let user = Option::from(user).expect("Default user shouldn't have place permisisons");

			let board = board.write().await;
			let board = board.as_ref().expect("Board went missing when creating a pixel");
			let place_attempt = board.try_place(
				// TODO: maybe accept option but make sure not to allow
				// undos etc for anon users
				&user,
				position,
				placement.color,
				connection.as_ref(),
			).await;

			match place_attempt {
				Ok(placement) => {
					let mut response = warp::reply::with_status(
						json(&placement).into_response(),
						StatusCode::CREATED,
					).into_response();

					let cooldown_info = board.user_cooldown_info(
						&user,
						connection.as_ref(),
					).await;

					#[allow(clippy::single_match)]
					match cooldown_info {
						Ok(cooldown_info) => {
							for (key, value) in cooldown_info.into_headers() {
								response = warp::reply::with_header(response, key, value)
									.into_response();
							}
						},
						Err(_) => {
							// TODO: not sure about this.
							// The resource *was* created, *but* the cooldown is not available.
							// I feel like sending the CREATED status is more important,
							// but other places currently return INTERNAL_SERVER_ERROR.

							//response = warp::reply::with_status(
							//	response,
							//	StatusCode::INTERNAL_SERVER_ERROR,
							//).into_response();
						},
					}

					response
				},
				Err(err) => err.into_response(),
			}
		})
}
