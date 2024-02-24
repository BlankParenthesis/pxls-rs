use std::sync::Arc;

use reqwest::StatusCode;
use warp::{Reply, Rejection};
use warp::Filter;
use serde::Deserialize;

use crate::filter::header::authorization::{Bearer, authorized};
use crate::filter::resource::database;
use crate::filter::resource::board::{self, PassableBoard};
use crate::filter::response::paginated_list::{PageToken, PaginationOptions, Page, DEFAULT_PAGE_ITEM_LIMIT, MAX_PAGE_ITEM_LIMIT};
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
		.and(authorized(users_db, Permission::BoardsPixelsList.into()))
		.and(database::connection(boards_db))
		.then(|board: PassableBoard, options: PaginationOptions<PageToken>, _, _, connection: BoardsConnection| async move {
			let page = options.page.unwrap_or_default();
			let limit = options
				.limit
				.unwrap_or(DEFAULT_PAGE_ITEM_LIMIT)
				.clamp(1, MAX_PAGE_ITEM_LIMIT);

			let board = board.read().await;
			let board = board.as_ref().expect("Board went missing when listing pixels");

			board.list_placements(page, limit, Order::Forward, &connection)
				.await
				.map(|(token, placements)| {
					let next = token.map(|token| format!(
						"/boards/{}/pixels?page={}&limit={}",
						board.id, token, limit
					));
					warp::reply::json(&Page {
						previous: None,
						items: placements.as_slice(),
						next,
					})
				})
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
		.and(authorized(users_db, Permission::BoardsPixelsList.into()))
		.and(database::connection(Arc::clone(&boards_db)))
		.then(|board: PassableBoard, position, _, _, connection: BoardsConnection| async move {
			let board = board.read().await;
			let board = board.as_ref().expect("Board went missing when getting a pixel");
			board.lookup(position, &connection).await?
				.map(|placement| warp::reply::json(&placement))
				.ok_or(StatusCode::NOT_FOUND)
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
		.and(authorized(users_db, Permission::BoardsPixelsPost.into()))
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
						warp::reply::json(&placement).into_response(),
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
