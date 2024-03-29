use std::sync::Arc;

use enumset::EnumSet;
use reqwest::StatusCode;
use warp::{Reply, Rejection};
use warp::Filter;
use serde::Deserialize;

use crate::config::CONFIG;
use crate::filter::header::authorization::{Bearer, self, PermissionsError};
use crate::filter::resource::database;
use crate::filter::resource::board::{self, PassableBoard};
use crate::permissions::Permission;
use crate::BoardDataMap;

use crate::database::{BoardsDatabase, BoardsConnection, UsersDatabase};

#[derive(Debug, Default, Deserialize, Clone, Copy)]
pub struct Overrides {
	#[serde(default)]
	pub cooldown: bool,
	#[serde(default)]
	pub color: bool,
	#[serde(default)]
	pub mask: bool,
}

impl From<Overrides> for EnumSet<Permission> {
	fn from(value: Overrides) -> Self {
		let mut permissions = EnumSet::empty();
		if value.cooldown {
			permissions.insert(Permission::BoardsPixelsOverrideCooldown);
		}
		if value.color {
			permissions.insert(Permission::BoardsPixelsOverrideColor);
		}
		if value.mask {
			permissions.insert(Permission::BoardsPixelsOverrideMask);
		}
		permissions
	}
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RangeEdit {
	Varied {
		position: u64,
		values: Vec<u8>,
	},
	Uniform {
		position: u64,
		value: u8,
		length: u64,
	},
}

#[derive(Debug, Deserialize)]
struct MassPlacementRequest {
	changes: Vec<RangeEdit>,
	#[serde(default)]
	overrides: Overrides,
}

pub fn patch(
	boards: BoardDataMap,
	boards_db: Arc<BoardsDatabase>,
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("boards")
		.and(board::path::read(&boards))
		.and(warp::path("pixels"))
		.and(warp::path::end())
		.and(warp::patch())
		.and(warp::body::json())
		.and(authorization::permissions(users_db))
		.and_then(|board, placement: MassPlacementRequest, permissions: EnumSet<Permission>, user, _| async move {
			let authorized = authorization::has_permissions(
				permissions,
				EnumSet::from(placement.overrides) | Permission::BoardsPixelsPatch
			);

			if authorized {
				Ok((board, placement, user))
			} else {
				Err(warp::reject::custom(PermissionsError::MissingPermission))
			}
		})
		.untuple_one()
		.and(database::connection(Arc::clone(&boards_db)))
		.then(|board: PassableBoard, placement: MassPlacementRequest, user: Option<Bearer>, connection: BoardsConnection| async move {
			let user = user.expect("Default user shouldn't have place permisisons");

			let changes = placement.changes.into_iter()
				.flat_map(|edit| {
					match edit {
						RangeEdit::Varied { position, values } => {
							values.into_iter()
								.enumerate()
								.map(|(i, v)| (position + i as u64, v))
								.collect::<Vec<_>>()
						},
						RangeEdit::Uniform { position, value, length } => {
							(0..length)
								.map(|i| (position + i, value))
								.collect::<Vec<_>>()
						},
					}
				})
				.collect::<Vec<_>>();

			let board = board.read().await;
			let board = board.as_ref().expect("Board went missing when creating a pixel");

			let place_attempt = board.mass_place(
				&user.id,
				&changes,
				placement.overrides,
				&connection,
			).await;

			match place_attempt {
				Ok((pixels_changed, timestamp)) => {
					let mut response = warp::reply::with_status(
						warp::reply::json(&pixels_changed).into_response(),
						StatusCode::CREATED,
					).into_response();

					if CONFIG.undo_deadline_seconds != 0 {
						response = warp::reply::with_header(
							response,
							"Pxls-Undo-Deadline",
							timestamp + CONFIG.undo_deadline_seconds,
						).into_response();
					}

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
