use std::sync::Arc;

use enumset::EnumSet;
use reqwest::StatusCode;
use warp::{Reply, Rejection};
use warp::Filter;
use serde::Deserialize;

use crate::board::PlacementPageToken;
use crate::config::CONFIG;
use crate::filter::header::authorization::{Bearer, authorized, self, PermissionsError};
use crate::filter::resource::database;
use crate::filter::resource::board::{self, PassableBoard};
use crate::filter::resource::filter::FilterRange;
use crate::filter::response::paginated_list::{
	PaginationOptions,
	DEFAULT_PAGE_ITEM_LIMIT,
	MAX_PAGE_ITEM_LIMIT,
};
use crate::permissions::Permission;
use crate::BoardDataMap;

use crate::database::{Order, BoardsDatabase, BoardsConnection, UsersDatabase, UsersConnection};
use crate::routes::board_moderation::boards::pixels::Overrides;

#[derive(Debug, Deserialize)]
pub struct PlacementFilter {
	#[serde(default)]
	pub position: FilterRange<u64>,
	#[serde(default)]
	pub color: FilterRange<u8>,
	#[serde(default)]
	pub timestamp: FilterRange<u32>,
	// FIXME: use uri and extract id, maybe change comparison code or change deserialize code
	pub user: Option<String>,
}


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
		.and(warp::query())
		.and(authorized(users_db, Permission::BoardsPixelsList.into()))
		.and(database::connection(boards_db))
		.then(|board: PassableBoard, options: PaginationOptions<PlacementPageToken>, filter: PlacementFilter, _, mut users_connection, connection: BoardsConnection| async move {
			let page = options.page;
			let limit = options
				.limit
				.unwrap_or(DEFAULT_PAGE_ITEM_LIMIT)
				.clamp(1, MAX_PAGE_ITEM_LIMIT);

			let board = board.read().await;
			let board = board.as_ref().expect("Board went missing when listing pixels");

			board.list_placements(
				page,
				limit,
				Order::Forward,
				filter,
				&connection,
				&mut users_connection,
			).await.map(|page| warp::reply::json(&page))
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
		.then(|board: PassableBoard, position, _, mut users_connection, connection: BoardsConnection| async move {
			let board = board.read().await;
			let board = board.as_ref().expect("Board went missing when getting a pixel");
			board.lookup(position, &connection, &mut users_connection).await?
				.map(|placement| warp::reply::json(&placement))
				.ok_or(StatusCode::NOT_FOUND)
		})
}

#[derive(Deserialize, Debug)]
struct PlacementRequest {
	color: u8,
	#[serde(default)]
	overrides: Overrides,
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
		.and(authorization::permissions(users_db))
		.and_then(|board, position, placement: PlacementRequest, permissions: EnumSet<Permission>, user, users_connection| async move {
			let authorized = authorization::has_permissions(
				permissions,
				EnumSet::from(placement.overrides) | Permission::BoardsPixelsPost,
			);

			if authorized {
				Ok((board, position, placement, user, users_connection))
			} else {
				Err(warp::reject::custom(PermissionsError::MissingPermission))
			}
		})
		.untuple_one()
		.and(database::connection(Arc::clone(&boards_db)))
		.then(|board: PassableBoard, position, placement: PlacementRequest, user: Option<Bearer>, mut users_connection: UsersConnection, connection: BoardsConnection| async move {
			let user = user.expect("Default user shouldn't have place permisisons");

			let board = board.read().await;
			let board = board.as_ref().expect("Board went missing when creating a pixel");
			let place_attempt = board.try_place(
				// TODO: maybe accept option but make sure not to allow
				// undos etc for anon users
				&user.id,
				position,
				placement.color,
				placement.overrides,
				&connection,
				&mut users_connection,
			).await;

			match place_attempt {
				Ok((cooldown, placement)) => {
					let mut response = warp::reply::with_status(
						warp::reply::json(&placement).into_response(),
						StatusCode::CREATED,
					).into_response();

					if CONFIG.undo_deadline_seconds != 0 {
						response = warp::reply::with_header(
							response,
							"Pxls-Undo-Deadline",
							placement.timestamp + CONFIG.undo_deadline_seconds,
						).into_response();
					}

					for (key, value) in cooldown.into_headers() {
						response = warp::reply::with_header(response, key, value)
							.into_response();
					}

					response
				},
				Err(err) => err.into_response(),
			}
		})
}
