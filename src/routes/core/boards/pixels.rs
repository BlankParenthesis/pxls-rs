use std::sync::Arc;

use reqwest::StatusCode;
use warp::reply::json;
use warp::{Reply, Rejection};
use warp::Filter;
use serde::Deserialize;

use crate::filter::header::authorization::{Bearer, authorized};
use crate::filter::resource::database;
use crate::filter::resource::board::{self, PassableBoard};
use crate::filter::response::paginated_list::{PageToken, PaginationOptions, Page};
use crate::permissions::Permission;
use crate::BoardDataMap;

use crate::database::{Order, BoardsDatabase, BoardsConnection, UsersDatabase};

pub fn list(
	boards: BoardDataMap,
	boards_db: Arc<BoardsDatabase>,
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("boards")
		.and(board::path::read(&boards))
		.and(warp::path("pixels"))
		.and(warp::path::end())
		.and(warp::get())
		.and(warp::query())
		.and(authorized(users_db, &[Permission::BoardsPixelsList]))
		.and(database::connection(boards_db))
		.then(|board: PassableBoard, options: PaginationOptions<PageToken>, _, _, connection: BoardsConnection| async move {
			let page = options.page.unwrap_or_default();
			let limit = options
				.limit
				.unwrap_or(10)
				.clamp(1, 100);

			let board = board.read().await;
			let board = board.as_ref().expect("Board went missing when listing pixels");

			let placements = board.list_placements(
				page,
				limit,
				Order::Forward,
				&connection
			).await;

			let (next, placements) = match placements {
				Ok((token, placements)) => {
					let token = token.map(|token| format!(
						"/boards/{}/pixels?page={}&limit={}",
						board.id, token, limit
					));
					(token, placements)
				},
				Err(err) => {
					return StatusCode::INTERNAL_SERVER_ERROR.into_response()
				},
			};

			json(&Page {
				previous: None,
				items: placements.as_slice(),
				next,
			})
			.into_response()
		})
}

pub fn get(
	boards: BoardDataMap,
	boards_db: Arc<BoardsDatabase>,
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("boards")
		.and(board::path::read(&boards))
		.and(warp::path("pixels"))
		.and(warp::path::param())
		.and(warp::path::end())
		.and(warp::get())
		.and(authorized(users_db, &[Permission::BoardsPixelsList]))
		.and(database::connection(Arc::clone(&boards_db)))
		.then(|board: PassableBoard, position, _, _, connection: BoardsConnection| async move {
			let board = board.read().await;
			let board = board.as_ref().expect("Board went missing when getting a pixel");
			match board.lookup(position, &connection).await {
				Ok(placement) => {
					placement
						.map(|placement| json(&placement).into_response())
						.unwrap_or_else(|| StatusCode::NOT_FOUND.into_response())
				},
				Err(err) => {
					StatusCode::INTERNAL_SERVER_ERROR.into_response()
				},
			}
		})
}

#[derive(Deserialize, Debug)]
pub struct PlacementRequest {
	pub color: u8,
}

pub fn post(
	boards: BoardDataMap,
	boards_db: Arc<BoardsDatabase>,
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("boards")
		.and(board::path::read(&boards))
		.and(warp::path("pixels"))
		.and(warp::path::param())
		.and(warp::path::end())
		.and(warp::post())
		.and(warp::body::json())
		.and(authorized(users_db, &[Permission::BoardsPixelsPost]))
		.and(database::connection(Arc::clone(&boards_db)))
		.then(|board: PassableBoard, position, placement: PlacementRequest, user: Option<Bearer>, _, connection: BoardsConnection| async move {
			let user = user.expect("Default user shouldn't have place permisisons");

			let board = board.read().await;
			let board = board.as_ref().expect("Board went missing when creating a pixel");
			let place_attempt = board.try_place(
				// TODO: maybe accept option but make sure not to allow
				// undos etc for anon users
				&user.id,
				position,
				placement.color,
				&connection,
			).await;

			match place_attempt {
				Ok(placement) => {
					let mut response = warp::reply::with_status(
						json(&placement).into_response(),
						StatusCode::CREATED,
					).into_response();

					let cooldown_info = board.user_cooldown_info(
						&user.id,
						&connection,
					).await;

					#[allow(clippy::single_match)]
					match cooldown_info {
						Ok(cooldown_info) => {
							for (key, value) in cooldown_info.into_headers() {
								response = warp::reply::with_header(response, key, value)
									.into_response();
							}
						},
						Err(err) => {
							// TODO: not sure about this.
							// The resource *was* created, *but* the cooldown is not available.
							// I feel like sending the CREATED status is more important,
							// but other places currently return INTERNAL_SERVER_ERROR.

							//response = StatusCode::INTERNAL_SERVER_ERROR
							//	.into_response();
						},
					}

					response
				},
				Err(err) => err.into_response(),
			}
		})
}
