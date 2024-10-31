use std::sync::Arc;

use enumset::EnumSet;
use reqwest::StatusCode;
use tokio::sync::RwLock;
use warp::{Reply, Rejection};
use warp::Filter;
use serde::Deserialize;

use crate::board::PlacementPageToken;
use crate::config::CONFIG;
use crate::filter::header::authorization::{self, authorized, has_permissions, permissions, Bearer, PermissionsError};
use crate::filter::resource::database;
use crate::filter::resource::board::{self, PassableBoard};
use crate::filter::resource::filter::FilterRange;
use crate::filter::response::paginated_list::{
	PaginationOptions,
	DEFAULT_PAGE_ITEM_LIMIT,
	MAX_PAGE_ITEM_LIMIT,
};
use crate::permissions::Permission;
use crate::routes::placement_statistics::users::calculate_stats;
use crate::BoardDataMap;

use crate::database::{BoardsConnection, BoardsDatabase, Order, User, UsersDatabase};
use crate::routes::board_moderation::boards::pixels::Overrides;
use crate::routes::core::{Connections, EventPacket};

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
		.and(permissions(users_db))
		.and_then(move |board, position, user_permissions: EnumSet<_>, _, connection| async move {
			if !has_permissions(user_permissions, Permission::BoardsPixelsGet.into()) {
				let error = PermissionsError::MissingPermission;
				return Err(Rejection::from(error));
			}
			Ok((board, position, user_permissions, connection))
		})
		.untuple_one()
		.and(database::connection(Arc::clone(&boards_db)))
		.then(|board: PassableBoard, position, user_permissions: EnumSet<_>, mut users_connection, connection: BoardsConnection| async move {
			let board = board.read().await;
			let board = board.as_ref().expect("Board went missing when getting a pixel");
			board.lookup(position, &connection, &mut users_connection).await?
				.map(|placement| {
					if user_permissions.contains(Permission::UsersGet) {
						warp::reply::json(&placement)
					} else {
						warp::reply::json(&serde_json::json!({
							"position": placement.position,
							"color": placement.color,
							"modified": placement.modified,
							"user": {
								"uri": placement.user.uri.to_string(),
								"view": None::<User>
							}
						}))
					}
				})
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
	events: Arc<RwLock<Connections>>,
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
		.then(move |board: PassableBoard, position, placement: PlacementRequest, user: Option<Bearer>, mut users_connection, boards_connection| {
			let events = events.clone();
			let boards = boards.clone();
			async move {
				let user = user.expect("Default user shouldn't have place permisisons");

				let place_attempt = {
					let board = board.read().await;
					let board = board.as_ref().expect("Board went missing when creating a pixel");
					board.try_place(
						// TODO: maybe accept option but make sure not to allow
						// undos etc for anon users
						&user.id,
						position,
						placement.color,
						placement.overrides,
						&boards_connection,
						&mut users_connection,
					).await
				};

				// This is required because read locking a rwlock twice in the
				// same thread can cause a deadlock and we're about to lock
				// boards in calculate_stats.
				drop(board);

				match place_attempt {
					Ok((cooldown, placement)) => {
						let stats = calculate_stats(
							user.id.clone(),
							&boards,
							&boards_connection,
							&mut users_connection,
						).await.map_err(|e| e.into_response())?;
						let user = Some(user.id);
						let packet = EventPacket::StatsUpdated { user, stats };

						let events = events.read().await;
						events.send(&packet).await;

						let mut response = warp::reply::with_status(
							warp::reply::json(&placement).into_response(),
							StatusCode::CREATED,
						).into_response();

						if CONFIG.undo_deadline_seconds != 0 {
							response = warp::reply::with_header(
								response,
								"Pxls-Undo-Deadline",
								placement.modified + CONFIG.undo_deadline_seconds,
							).into_response();
						}

						for (key, value) in cooldown.into_headers() {
							response = warp::reply::with_header(response, key, value)
								.into_response();
						}

						Ok(response)
					},
					Err(err) => Err(err.into_response()),
				}
			}
		})
}
