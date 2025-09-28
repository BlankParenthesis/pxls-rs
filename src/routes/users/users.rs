use std::sync::Arc;

use serde::Deserialize;
use tokio::sync::RwLock;
use warp::http::StatusCode;
use warp::{
	Filter,
	Reply,
	Rejection,
};

use crate::config::CONFIG;
use crate::routes::core::{EventPacket, Connections};
use crate::database::{BoardsConnection, BoardsDatabase, User, UserSpecifier};
use crate::filter::response::paginated_list::PaginationOptions;
use crate::filter::header::authorization::{self, PermissionsError};
use crate::filter::resource::filter::FilterRange;
use crate::filter::response::reference::Reference;
use crate::permissions::Permission;

#[derive(Deserialize, Debug)]
pub struct UserFilter {
	pub name: Option<String>,
	#[serde(default)]
	pub created_at: FilterRange<i64>,
}

pub fn list(
	db: Arc<BoardsDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("users")
		.and(warp::path::end())
		.and(warp::get())
		.and(warp::query())
		.and(warp::query())
		.and(authorization::authorized(db, Permission::UsersList.into()))
		.then(move |pagination: PaginationOptions<_>, filter: UserFilter, _, connection: BoardsConnection| async move {
			let page = pagination.page;
			let limit = pagination.limit
				.unwrap_or(CONFIG.default_page_item_limit)
				.clamp(1, CONFIG.max_page_item_limit); // TODO: maybe raise upper limit
			
			connection.list_users(page, limit, filter).await
				.map(|page| warp::reply::json(&page))
		})
}

pub fn get(
	db: Arc<BoardsDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	let permissions = Permission::UsersGet.into();

	warp::path("users")
		.and(warp::path::param())
		.and(warp::path::end())
		.and(warp::get())
		.and(authorization::permissions(db))
		.and_then(move |user: UserSpecifier, user_permissions, requester: Option<User>, connection| async move {
			let is_current_user = requester.as_ref()
				.map(|u| u.specifier() == user)
				.unwrap_or(false);

			let check = if is_current_user {
				authorization::has_permissions_current
			} else {
				authorization::has_permissions
			};

			if check(user_permissions, permissions) {
				Ok((user, connection))
			} else {
				Err(warp::reject::custom(PermissionsError::MissingPermission))
			}
		})
		.untuple_one()
		.then(move |user: UserSpecifier, connection: BoardsConnection| async move {
			connection.get_user(&user).await
				.map(|user| warp::reply::json(&user))
		})
}

pub fn current(
	db: Arc<BoardsDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("users")
		.and(warp::path("current"))
		.and(warp::path::end())
		.and(warp::get())
		.and(authorization::authorized(db, Permission::UsersCurrentGet.into()))
		.then(|user: Option<User>, _| async move {
			if let Some(user) = user {
				Ok(Reference::from(user).reply())
			} else {
				Err(StatusCode::UNAUTHORIZED)
			}
		})
}

#[derive(Debug, Deserialize)]
struct UserUpdate {
	name: String,
}

pub fn patch(
	db: Arc<BoardsDatabase>,
	events: Arc<RwLock<Connections>>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	let permissions = Permission::UsersGet | Permission::UsersPatch;

	warp::path("users")
		.and(warp::path::param())
		.and(warp::path::end())
		.and(warp::patch())
		.and(warp::body::json())
		.and(authorization::permissions(db))
		.and_then(move |user: UserSpecifier, update, user_permissions, requester: Option<User>, connection| async move {
			let is_current_user = requester.as_ref()
				.map(|u| u.specifier() == user)
				.unwrap_or(false);

			let check = if is_current_user {
				authorization::has_permissions_current
			} else {
				authorization::has_permissions
			};

			if check(user_permissions, permissions) {
				Ok((user, update, connection))
			} else {
				Err(warp::reject::custom(PermissionsError::MissingPermission))
			}
		})
		.untuple_one()
		.then(move |user: UserSpecifier, update: UserUpdate, connection: BoardsConnection| {
			let events = events.clone();
			async move {
				// FIXME: validate username

				let user = connection.update_user(&user, &update.name).await?
					.ok_or(StatusCode::NOT_FOUND)?;

				let reference = Reference::from(user);
				let packet = EventPacket::UserUpdated {
					user: reference.clone(),
				};
				events.read().await.send(&packet).await;
				Ok::<_, StatusCode>(reference.created()) // TODO: is created correct?
			}
		})
}

pub fn delete(
	db: Arc<BoardsDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	let permissions = Permission::UsersGet | Permission::UsersDelete;

	warp::path("users")
		.and(warp::path::param())
		.and(warp::path::end())
		.and(warp::delete())
		.and(authorization::permissions(db))
		.and_then(move |user: UserSpecifier, user_permissions, requester: Option<User>, connection| async move {
			let is_current_user = requester.as_ref()
				.map(|u| u.specifier() == user)
				.unwrap_or(false);

			let check = if is_current_user {
				authorization::has_permissions_current
			} else {
				authorization::has_permissions
			};

			if check(user_permissions, permissions) {
				Ok((user, connection))
			} else {
				Err(warp::reject::custom(PermissionsError::MissingPermission))
			}
		})
		.untuple_one()
		.then(move |user: UserSpecifier, connection: BoardsConnection| async move {
			connection.delete_user(&user).await
				.map(|_| StatusCode::NO_CONTENT)
		})
}
