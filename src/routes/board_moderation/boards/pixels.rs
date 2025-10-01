use std::sync::Arc;

use enumset::EnumSet;
use reqwest::StatusCode;
use warp::{Reply, Rejection};
use warp::Filter;
use serde::Deserialize;

use crate::config::CONFIG;
use crate::database::{Database, DbConn, PlacementListSpecifier, Specifier, User};
use crate::filter::header::authorization::{self, PermissionsError};
use crate::permissions::Permission;
use crate::BoardDataMap;

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
	boards_db: Arc<Database>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	PlacementListSpecifier::path()
		.and(warp::patch())
		.and(warp::body::json())
		.and(authorization::permissions(boards_db))
		.and_then(|placement: PlacementListSpecifier, patch: MassPlacementRequest, permissions: EnumSet<Permission>, user, connection| async move {
			let authorized = authorization::has_permissions(
				permissions,
				EnumSet::from(patch.overrides) | Permission::BoardsPixelsPatch
			);

			if authorized {
				Ok((placement, patch, user, connection))
			} else {
				Err(warp::reject::custom(PermissionsError::MissingPermission))
			}
		})
		.untuple_one()
		.then(move |placement: PlacementListSpecifier, patch: MassPlacementRequest, user: Option<User>, connection: DbConn| {
			let boards = boards.clone();
			async move {
				let user = user.expect("Default user shouldn't have place permisisons");
				
				let changes = patch.changes.into_iter()
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
				
				let boards = boards.read().await;
				let board = boards.get(&placement.board())
					.ok_or(StatusCode::NOT_FOUND)?;
				let mut board = board.write().await;
				let board = board.as_mut()
					.expect("Board went missing when creating a pixel");
	
				let (pixels_changed, timestamp) = board.mass_place(
					&user,
					&changes,
					patch.overrides,
					&connection,
				).await?;
	
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

				let cooldown_info = board.user_cooldown_info(user.specifier()).await;

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

				Ok::<_, StatusCode>(response)
			}
		})
}
