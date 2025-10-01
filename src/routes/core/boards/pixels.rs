use std::sync::Arc;

use enumset::EnumSet;
use reqwest::StatusCode;
use tokio::sync::RwLock;
use warp::{Reply, Rejection};
use warp::Filter;
use serde::Deserialize;

use crate::config::CONFIG;
use crate::filter::header::authorization::{authorized, has_permissions, permissions, PermissionsError};
use crate::filter::resource::board::{self, PassableBoard};
use crate::filter::resource::filter::FilterRange;
use crate::filter::response::paginated_list::PaginationOptions;
use crate::permissions::Permission;
use crate::routes::placement_statistics::users::calculate_stats;
use crate::BoardDataMap;

use crate::database::{Database, DbConn, Order, PlacementPageToken, PlacementSpecifier, Specifier, User};
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
	db: Arc<Database>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("boards")
		.and(board::path::read(&boards))
		.and(warp::path("pixels"))
		.and(warp::path::end())
		.and(warp::get())
		.and(warp::query())
		.and(warp::query())
		.and(authorized(db, Permission::BoardsPixelsList.into()))
		.then(|board: PassableBoard, options: PaginationOptions<PlacementPageToken>, filter: PlacementFilter, _, connection: DbConn| async move {
			let page = options.page;
			let limit = options
				.limit
				.unwrap_or(CONFIG.default_page_item_limit)
				.clamp(1, CONFIG.max_page_item_limit);

			let board = board.read().await;
			let board = board.as_ref().expect("Board went missing when listing pixels");

			board.list_placements(
				page,
				limit,
				Order::Forward,
				filter,
				&connection,
			).await.map(|page| warp::reply::json(&page))
		})
}

pub fn get(
	boards: BoardDataMap,
	db: Arc<Database>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	PlacementSpecifier::path()
		.and(warp::path::end())
		.and(warp::get())
		.and(permissions(db))
		.and_then(move |placement, user_permissions: EnumSet<_>, _, connection| async move {
			if !has_permissions(user_permissions, Permission::BoardsPixelsGet.into()) {
				let error = PermissionsError::MissingPermission;
				return Err(Rejection::from(error));
			}
			Ok((placement, user_permissions, connection))
		})
		.untuple_one()
		.then(move |placement: PlacementSpecifier, user_permissions: EnumSet<_>, connection: DbConn| {
			let boards = boards.clone();
			async move {
				let boards = boards.read().await;
				let board = boards.get(&placement.board())
					.ok_or(StatusCode::NOT_FOUND)?;
				let board = board.read().await;
				let board = board.as_ref().expect("Board went missing when getting a pixel");
				board.lookup(&placement, &connection).await?
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
			}
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
	db: Arc<Database>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	PlacementSpecifier::path()
		.and(warp::post())
		.and(warp::body::json())
		.and(permissions(db))
		.and_then(|placement: PlacementSpecifier, post: PlacementRequest, permissions: EnumSet<Permission>, user, connection| async move {
			let authorized = has_permissions(
				permissions,
				EnumSet::from(post.overrides) | Permission::BoardsPixelsPost,
			);

			if authorized {
				Ok((placement, post, user, connection))
			} else {
				Err(warp::reject::custom(PermissionsError::MissingPermission))
			}
		})
		.untuple_one()
		.then(move |placement: PlacementSpecifier, post: PlacementRequest, user: Option<User>, connection| {
			let events = events.clone();
			let boards = boards.clone();
			async move {
				let user = user.expect("Default user shouldn't have place permisisons");
				
				let (cooldown, place) = {
					let boards = boards.read().await;
					let board = boards.get(&placement.board())
						.ok_or(StatusCode::NOT_FOUND)?;
					let board = board.read().await;
					let board = board.as_ref().expect("Board went missing when creating a pixel");
					let (cooldown, placement) = board.try_place(
						// TODO: maybe accept option but make sure not to allow
						// undos etc for anon users
						&user,
						&placement,
						post.color,
						post.overrides,
						&connection,
					).await?;
					(cooldown, placement)
				};

				let stats = calculate_stats(&user, &boards).await?;
				let user = Some(*user.specifier());
				let packet = EventPacket::StatsUpdated { user, stats };

				let events = events.read().await;
				events.send(&packet).await;

				let mut response = warp::reply::with_status(
					warp::reply::json(&place).into_response(),
					StatusCode::CREATED,
				).into_response();

				if CONFIG.undo_deadline_seconds != 0 {
					response = warp::reply::with_header(
						response,
						"Pxls-Undo-Deadline",
						place.modified + CONFIG.undo_deadline_seconds,
					).into_response();
				}

				response = warp::reply::with_header(
					response,
					"Location",
					placement.to_uri().to_string(),
				).into_response();

				for (key, value) in cooldown.into_headers() {
					response = warp::reply::with_header(response, key, value)
						.into_response();
				}

				Ok::<_, StatusCode>(response)
			}
		})
}
