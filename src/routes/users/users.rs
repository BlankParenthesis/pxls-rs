use std::sync::Arc;

use serde::Deserialize;
use warp::{
	http::{StatusCode, Uri},
	Filter,
	Reply,
	Rejection,
	path::Tail,
};

use crate::filter::response::paginated_list::{PaginationOptions, Page, DEFAULT_PAGE_ITEM_LIMIT, MAX_PAGE_ITEM_LIMIT};
use crate::filter::response::reference::Reference;
use crate::filter::header::authorization::{self, Bearer, PermissionsError};
use crate::permissions::Permission;
use crate::database::{UsersDatabase, UsersConnection};

pub fn list(
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("users")
		.and(warp::path::end())
		.and(warp::get())
		.and(warp::query())
		.and(authorization::authorized(users_db, Permission::UsersList.into()))
		.then(move |pagination: PaginationOptions<String>, _, mut connection: UsersConnection| async move {
			let page = pagination.page;
			let limit = pagination.limit
				.unwrap_or(DEFAULT_PAGE_ITEM_LIMIT)
				.clamp(1, MAX_PAGE_ITEM_LIMIT); // TODO: maybe raise upper limit
			
			connection.list_users(page, limit).await
				.map(|(page_token, users)| {
					let references = users.iter()
						.map(Reference::from)
						.collect::<Vec<_>>();

					let page = Page {
						items: &references[..],
						next: page_token.map(|p| format!("/users?limit={}&page={}", limit, p)),
						// TODO: either find some magical way to generate this or
						// change the spec
						previous: None,
					};

					warp::reply::json(&page)
				})
		})
}

pub fn get(
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	let permissions = Permission::UsersGet | Permission::UsersPatch;

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
		.and(warp::path::tail())
		.and(warp::get())
		.and(authorization::authorized(users_db, Permission::UsersCurrentGet.into()))
		.then(|tail: Tail, user: Option<Bearer>, _| async move {
			if let Some(uid) = user.map(|b| b.id) {
				let location = format!("/users/{}/{}", uid, tail.as_str())
					.parse::<Uri>().unwrap();
				Ok(warp::redirect::temporary(location))
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
		.then(move |uid: String, update: UserUpdate, mut connection: UsersConnection| async move {
			// TODO: validate username

			connection.update_user(&uid, &update.name).await?;
			connection.get_user(&uid).await
				.map(|user| warp::reply::json(&user))
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

// TODO: socket stuff 