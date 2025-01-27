use std::sync::Arc;

use reqwest::StatusCode;
use serde::{Serialize, Deserialize};
use tokio::sync::RwLock;
use warp::{
	Filter,
	Reply,
	Rejection,
};
use warp::http::Uri;

use crate::config::CONFIG;
use crate::filter::resource::filter::FilterRange;
use crate::routes::core::{Connections, EventPacket};
use crate::database::BoardsDatabaseError;
use crate::filter::response::paginated_list::{
	PaginationOptions,
	PageToken
};
use crate::filter::header::authorization::{self, Bearer, PermissionsError};
use crate::filter::response::reference::Reference;
use crate::filter::resource::database;
use crate::permissions::Permission;
use crate::database::{UsersDatabase, UsersConnection, User, BoardsDatabase, BoardsConnection};

#[derive(Debug, Serialize, Clone)]
pub struct Ban {
	pub created_at: u64,
	pub expires_at: Option<u64>,
	pub issuer: Option<Reference<User>>,
	pub reason: Option<String>,
}

impl Ban {
	pub fn uri(id: i32, user: &str) -> Uri {
		format!("/users/{}/bans/{}", user, id).parse().unwrap()
	}
}

#[derive(Debug, Default, Deserialize)]
pub struct BanPageToken(pub u32);

impl PageToken for BanPageToken {}

#[derive(Debug, Deserialize)]
pub struct BanFilter {
	#[serde(default)]
	pub created_at: FilterRange<u64>,
	#[serde(default)]
	pub expires_at: FilterRange<u64>,
	// TODO: issuer, reason
}

pub fn list(
	boards_db: Arc<BoardsDatabase>,
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	let permissions = Permission::UsersBansList.into();

	warp::path("users")
		.and(warp::path::param())
		.and(warp::path::path("bans"))
		.and(warp::path::end())
		.and(warp::get())
		.and(warp::query())
		.and(warp::query())
		.and(authorization::permissions(users_db))
		.and_then(move |uid: String, pagination, filter, user_permissions, bearer: Option<Bearer>, connection| async move {
			let is_current_user = bearer.as_ref()
				.map(|bearer| bearer.id == uid)
				.unwrap_or(false);

			let check = if is_current_user {
				authorization::has_permissions_current
			} else {
				authorization::has_permissions
			};

			if check(user_permissions, permissions) {
				Ok((uid, pagination, filter, connection))
			} else {
				Err(warp::reject::custom(PermissionsError::MissingPermission))
			}
		})
		.untuple_one()
		.and(database::connection(boards_db))
		.then(move |uid: String, pagination: PaginationOptions<BanPageToken>, filter, mut users_connection, boards_connection: BoardsConnection| async move {
			let page = pagination.page;
			let limit = pagination.limit
				.unwrap_or(CONFIG.default_page_item_limit)
				.clamp(1, CONFIG.max_page_item_limit);

			boards_connection.list_user_bans(
				&uid,
				page,
				limit,
				filter,
				&mut users_connection,
			).await.map(|page| warp::reply::json(&page))
		})
}

pub fn get(
	boards_db: Arc<BoardsDatabase>,
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	let permissions = Permission::UsersBansGet.into();

	warp::path("users")
		.and(warp::path::param())
		.and(warp::path::path("bans"))
		.and(warp::path::param())
		.and(warp::path::end())
		.and(warp::get())
		.and(authorization::permissions(users_db))
		.and_then(move |uid: String, ban_id, user_permissions, bearer: Option<Bearer>, connection| async move {
			let is_current_user = bearer.as_ref()
				.map(|bearer| bearer.id == uid)
				.unwrap_or(false);

			let check = if is_current_user {
				authorization::has_permissions_current
			} else {
				authorization::has_permissions
			};

			if check(user_permissions, permissions) {
				Ok((uid, ban_id, connection))
			} else {
				Err(warp::reject::custom(PermissionsError::MissingPermission))
			}
		})
		.untuple_one()
		.and(database::connection(boards_db))
		.then(move |uid: String, ban_id: usize, mut users_connection, boards_connection: BoardsConnection| async move {
			boards_connection.get_ban(
				ban_id, 
				&uid, 
				&mut users_connection,
			).await.map(|page| {
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
	boards_db: Arc<BoardsDatabase>,
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("users")
		.and(warp::path::param())
		.and(warp::path::path("bans"))
		.and(warp::path::end())
		.and(warp::post())
		.and(warp::body::json())
		.and(authorization::authorized(users_db, Permission::UsersBansGet | Permission::UsersBansPost))
		.and(database::connection(boards_db))
		.then(move |uid: String, ban: BanPost, user: Option<Bearer>, mut users_connection: UsersConnection, boards_connection: BoardsConnection| {
			let events = Arc::clone(&events);
			async move {				
				let ban = boards_connection.create_ban(
					&uid,
					user.map(|b| b.id),
					ban.reason,
					ban.expires_at,
					&mut users_connection,
				).await.map_err(Reply::into_response)?;

				let packet = EventPacket::UserBanCreated {
					user: uid,
					ban: ban.clone(),
				};
				let events = events.write().await;
				events.send(&packet).await;
				
				Ok::<_, warp::reply::Response>(ban.created())
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
	boards_db: Arc<BoardsDatabase>,
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("users")
		.and(warp::path::param())
		.and(warp::path::path("bans"))
		.and(warp::path::param())
		.and(warp::path::end())
		.and(warp::patch())
		.and(warp::body::json())
		.and(authorization::authorized(users_db, Permission::UsersBansGet | Permission::UsersBansPatch))
		.and(database::connection(boards_db))
		.then(move |uid: String, ban_id: usize, ban: BanPatch, _: Option<Bearer>, mut users_connection: UsersConnection, boards_connection: BoardsConnection| {
			let events = Arc::clone(&events);
			async move {
				let ban = boards_connection.edit_ban(
					ban_id,
					&uid,
					ban.reason,
					ban.expires_at,
					&mut users_connection,
				).await.map_err(Reply::into_response)?;

				let packet = EventPacket::UserBanUpdated {
					user: uid,
					ban: ban.clone(),
				};
				let events = events.write().await;
				events.send(&packet).await;
				
				Ok::<_, warp::reply::Response>(warp::reply::json(&ban))
			}
		})
}

pub fn delete(
	events: Arc<RwLock<Connections>>,
	boards_db: Arc<BoardsDatabase>,
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("users")
		.and(warp::path::param())
		.and(warp::path::path("bans"))
		.and(warp::path::param())
		.and(warp::path::end())
		.and(warp::delete())
		.and(authorization::authorized(users_db, Permission::UsersBansGet | Permission::UsersBansDelete))
		.and(database::connection(boards_db))
		.then(move |user: String, id: usize, _: Option<Bearer>, mut users_connection: UsersConnection, boards_connection: BoardsConnection| {
			let events = Arc::clone(&events);
			async move {
				let was_deleted = boards_connection.delete_ban(
					id,
					&user,
					&mut users_connection,
				).await?;

				if was_deleted {
					let packet = EventPacket::UserBanDeleted {
						ban: format!("/users/{}/bans/{}", &user, id)
							.parse::<Uri>().unwrap(),
						user,
					};
					let events = events.write().await;
					events.send(&packet).await;

					Ok::<_, BoardsDatabaseError>(StatusCode::NO_CONTENT)
				} else {
					Ok::<_, BoardsDatabaseError>(StatusCode::NOT_FOUND)
				}
			}
		})
}
