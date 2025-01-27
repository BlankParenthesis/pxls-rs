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
use crate::database::{User, UsersDatabaseError};
use crate::filter::response::paginated_list::PaginationOptions;
use crate::filter::header::authorization::{self, Bearer, PermissionsError};
use crate::filter::resource::filter::FilterRange;
use crate::filter::response::reference::Reference;
use crate::permissions::Permission;
use crate::database::{UsersDatabase, UsersConnection, LdapPageToken};

#[derive(Deserialize, Debug)]
pub struct UserFilter {
	pub name: Option<String>,
	#[serde(default)]
	pub created_at: FilterRange<i64>,
}

pub fn list(
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("users")
		.and(warp::path::end())
		.and(warp::get())
		.and(warp::query())
		.and(warp::query())
		.and(authorization::authorized(users_db, Permission::UsersList.into()))
		.then(move |pagination: PaginationOptions<LdapPageToken>, filter: UserFilter, _, mut connection: UsersConnection| async move {
			let page = pagination.page;
			let limit = pagination.limit
				.unwrap_or(CONFIG.default_page_item_limit)
				.clamp(1, CONFIG.max_page_item_limit); // TODO: maybe raise upper limit
			
			connection.list_users(page, limit, filter).await
				.map(|page| warp::reply::json(&page))
		})
}

pub fn get(
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	let permissions = Permission::UsersGet.into();

	warp::path("users")
		.and(warp::path::param())
		.and(warp::path::end())
		.and(warp::get())
		.and(authorization::permissions(users_db))
		.and_then(move |uid: String, user_permissions, bearer: Option<Bearer>, connection| async move {
			let is_current_user = bearer.as_ref()
				.map(|bearer| bearer.id == uid)
				.unwrap_or(false);

			let check = if is_current_user {
				authorization::has_permissions_current
			} else {
				authorization::has_permissions
			};

			if check(user_permissions, permissions) {
				Ok((uid, connection))
			} else {
				Err(warp::reject::custom(PermissionsError::MissingPermission))
			}
		})
		.untuple_one()
		.then(move |uid: String, mut connection: UsersConnection| async move {
			connection.get_user(&uid).await
				.map(|user| warp::reply::json(&user))
		})
}

pub fn current(
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("users")
		.and(warp::path("current"))
		.and(warp::path::end())
		.and(warp::get())
		.and(authorization::authorized(users_db, Permission::UsersCurrentGet.into()))
		.then(|user: Option<Bearer>, mut connection: UsersConnection| async move {
			if let Some(uid) = user.map(|b| b.id) {
				connection.get_user(&uid).await
					.map(|user| Reference::new(User::uri(&uid), &user).reply())
					.map_err(StatusCode::from)
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
	users_db: Arc<UsersDatabase>,
	events: Arc<RwLock<Connections>>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	let permissions = Permission::UsersGet | Permission::UsersPatch;

	warp::path("users")
		.and(warp::path::param())
		.and(warp::path::end())
		.and(warp::patch())
		.and(warp::body::json())
		.and(authorization::permissions(users_db))
		.and_then(move |uid: String, update, user_permissions, bearer: Option<Bearer>, connection| async move {
			let is_current_user = bearer.as_ref()
				.map(|bearer| bearer.id == uid)
				.unwrap_or(false);

			let check = if is_current_user {
				authorization::has_permissions_current
			} else {
				authorization::has_permissions
			};

			if check(user_permissions, permissions) {
				Ok((uid, update, connection))
			} else {
				Err(warp::reject::custom(PermissionsError::MissingPermission))
			}
		})
		.untuple_one()
		.then(move |uid: String, update: UserUpdate, mut connection: UsersConnection| {
			let events = events.clone();
			async move {
				// FIXME: validate username

				connection.update_user(&uid, &update.name).await?;
				let user = connection.get_user(&uid).await?;
				let reference = Reference::new(User::uri(&uid), user);

				let packet = EventPacket::UserUpdated {
					user_id: uid,
					user: reference.clone(),
				};
				events.read().await.send(&packet).await;
				Ok::<_, UsersDatabaseError>(reference.created()) // TODO: is created correct?
			}
		})
}

pub fn delete(
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	let permissions = Permission::UsersGet | Permission::UsersDelete;

	warp::path("users")
		.and(warp::path::param())
		.and(warp::path::end())
		.and(warp::delete())
		.and(authorization::permissions(users_db))
		.and_then(move |uid: String, user_permissions, bearer: Option<Bearer>, connection| async move {
			let is_current_user = bearer.as_ref()
				.map(|bearer| bearer.id == uid)
				.unwrap_or(false);

			let check = if is_current_user {
				authorization::has_permissions_current
			} else {
				authorization::has_permissions
			};

			if check(user_permissions, permissions) {
				Ok((uid, connection))
			} else {
				Err(warp::reject::custom(PermissionsError::MissingPermission))
			}
		})
		.untuple_one()
		.then(move |uid: String, mut connection: UsersConnection| async move {
			connection.delete_user(&uid).await
				.map(|_| StatusCode::NO_CONTENT)
		})
}
