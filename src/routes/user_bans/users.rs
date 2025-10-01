use std::sync::Arc;

use reqwest::StatusCode;
use serde::Deserialize;
use tokio::sync::RwLock;
use warp::{Filter, Reply, Rejection};

use crate::config::CONFIG;
use crate::filter::resource::filter::FilterRange;
use crate::filter::response::reference::Reference;
use crate::routes::core::{Connections, EventPacket};
use crate::filter::response::paginated_list::PaginationOptions;
use crate::filter::header::authorization::{self, PermissionsError};
use crate::permissions::Permission;
use crate::database::{Specifier, Database, DbConn, User, BanSpecifier, BanListSpecifier};

#[derive(Debug, Deserialize)]
pub struct BanFilter {
	#[serde(default)]
	pub created_at: FilterRange<u64>,
	#[serde(default)]
	pub expires_at: FilterRange<u64>,
	// TODO: issuer, reason
}

pub fn list(
	db: Arc<Database>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	let permissions = Permission::UsersBansList.into();

	BanListSpecifier::path()
		.and(warp::get())
		.and(warp::query())
		.and(warp::query())
		.and(authorization::permissions(db))
		.and_then(move |user: BanListSpecifier, pagination, filter, user_permissions, requester: Option<User>, connection| async move {
			let is_current_user = requester.as_ref()
				.map(|u| *u.specifier() == user.user())
				.unwrap_or(false);

			let check = if is_current_user {
				authorization::has_permissions_current
			} else {
				authorization::has_permissions
			};

			if check(user_permissions, permissions) {
				Ok((user, pagination, filter, connection))
			} else {
				Err(warp::reject::custom(PermissionsError::MissingPermission))
			}
		})
		.untuple_one()
		.then(move |user: BanListSpecifier, pagination: PaginationOptions<_>, filter, connection: DbConn| async move {
			let page = pagination.page;
			let limit = pagination.limit
				.unwrap_or(CONFIG.default_page_item_limit)
				.clamp(1, CONFIG.max_page_item_limit);

			connection.list_bans(&user, page, limit, filter).await
				.map(|page| warp::reply::json(&page))
		})
}

pub fn get(
	db: Arc<Database>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	let permissions = Permission::UsersBansGet.into();
	
	BanSpecifier::path()
		.and(warp::get())
		.and(authorization::permissions(db))
		.and_then(move |ban: BanSpecifier, user_permissions, requester: Option<User>, connection| async move {
			let is_current_user = requester.as_ref()
				.map(|u| *u.specifier() == ban.user())
				.unwrap_or(false);

			let check = if is_current_user {
				authorization::has_permissions_current
			} else {
				authorization::has_permissions
			};

			if check(user_permissions, permissions) {
				Ok((ban, connection))
			} else {
				Err(warp::reject::custom(PermissionsError::MissingPermission))
			}
		})
		.untuple_one()
		.then(move |ban: BanSpecifier, connection: DbConn| async move {
			connection.get_ban(&ban).await
				.map(|page| {
					if let Some(page) = page {
						warp::reply::json(&page).into_response()
					} else {
						StatusCode::NOT_FOUND.into_response()
					}
				})
		})
}

#[derive(Debug, Deserialize)]
struct BanPost {
	expires_at: Option<u64>,
	reason: Option<String>,
}

pub fn post(
	events: Arc<RwLock<Connections>>,
	db: Arc<Database>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	BanListSpecifier::path()
		.and(warp::post())
		.and(warp::body::json())
		.and(authorization::authorized(db, Permission::UsersBansGet | Permission::UsersBansPost))
		.then(move |list: BanListSpecifier, post: BanPost, issuer: Option<User>, connection: DbConn| {
			let events = Arc::clone(&events);
			async move {
				let ban = connection.create_ban(
					&list,
					issuer.map(|i| *i.specifier()).as_ref(),
					post.reason,
					post.expires_at,
				).await?;
				
				let ban = Reference::from(ban);

				let packet = EventPacket::UserBanCreated {
					user: list.user(),
					ban: ban.clone(),
				};
				let events = events.write().await;
				events.send(&packet).await;
				
				Ok::<_, StatusCode>(ban.created())
			}
		})
}

#[derive(Debug, Deserialize)]
struct BanPatch {
	#[serde(default, with = "serde_with::rust::double_option")]
	reason: Option<Option<String>>,
	#[serde(default, with = "serde_with::rust::double_option")]
	expires_at: Option<Option<u64>>,
}

pub fn patch(
	events: Arc<RwLock<Connections>>,
	db: Arc<Database>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	BanSpecifier::path()
		.and(warp::patch())
		.and(warp::body::json())
		.and(authorization::authorized(db, Permission::UsersBansGet | Permission::UsersBansPatch))
		.then(move |ban: BanSpecifier, patch: BanPatch, _, connection: DbConn| {
			let events = Arc::clone(&events);
			async move {
				let edit = connection.edit_ban(
					&ban,
					patch.reason,
					patch.expires_at,
				).await?
				.ok_or(StatusCode::NOT_FOUND)?;

				let packet = EventPacket::UserBanUpdated {
					user: ban.user(),
					ban: Reference::from(edit.clone()),
				};
				let events = events.write().await;
				events.send(&packet).await;
				
				Ok::<_, StatusCode>(warp::reply::json(&edit))
			}
		})
}

pub fn delete(
	events: Arc<RwLock<Connections>>,
	db: Arc<Database>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	BanSpecifier::path()
		.and(warp::delete())
		.and(authorization::authorized(db, Permission::UsersBansGet | Permission::UsersBansDelete))
		.then(move |ban: BanSpecifier, _, connection: DbConn| {
			let events = Arc::clone(&events);
			async move {
				let was_deleted = connection.delete_ban(&ban).await?;

				if was_deleted {
					let packet = EventPacket::UserBanDeleted {
						ban,
						user: ban.user(),
					};
					let events = events.write().await;
					events.send(&packet).await;

					Ok::<_, StatusCode>(StatusCode::NO_CONTENT)
				} else {
					Ok::<_, StatusCode>(StatusCode::NOT_FOUND)
				}
			}
		})
}
