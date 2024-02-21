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
use crate::database::{UpdateError, DeleteError};
use crate::filter::response::reference::Reference;
use crate::filter::header::authorization::{self, Bearer, UsersDBError, PermissionsError};
use crate::filter::resource::database;
use crate::permissions::Permission;
use crate::database::{UsersDatabase, UsersConnection, FetchError};

pub fn list(
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("users")
		.and(warp::path::end())
		.and(warp::get())
		.and(warp::query())
		.and(authorization::authorized(users_db, &[Permission::UsersList]))
		.then(move |pagination: PaginationOptions<String>, _, mut connection: UsersConnection| async move {
			let page = pagination.page;
			let limit = pagination.limit
				.unwrap_or(DEFAULT_PAGE_ITEM_LIMIT)
				.clamp(1, MAX_PAGE_ITEM_LIMIT); // TODO: maybe raise upper limit
			
			match connection.list_users(page, limit).await {
				Ok((page_token, users)) => {
					let references = users.iter()
						.map(|u| Reference {
							uri: format!("/users/{}", u.id).parse().unwrap(),
							view: u,
						})
						.collect::<Vec<_>>();

					let page = Page {
						items: &references[..],
						next: page_token.map(|p| format!("/users?limit={}&page={}", limit, p)),
						// TODO: either find some magical way to generate this or
						// change the spec
						previous: None,
					};

					warp::reply::json(&page).into_response()
				},
				Err(FetchError::InvalidPage) => {
					StatusCode::BAD_REQUEST.into_response()
				},
				Err(err) => {
					StatusCode::INTERNAL_SERVER_ERROR.into_response()
				},
			}
		})
}

pub fn get(
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("users")
		.and(warp::path::param())
		.and(warp::path::end())
		.and(warp::get())
		.and(authorization::bearer())
		.and(database::connection(users_db))
		.and_then(|uid: String, bearer: Option<Bearer>, mut connection: UsersConnection| async {
			let user_permissions = match bearer {
				Some(ref user) => {
					connection.user_permissions(&user.id).await
						.map_err(UsersDBError::Raw)
						.map_err(Rejection::from)?
				}
				None => Permission::defaults(),
			};

			let is_current_user = bearer.as_ref()
				.map(|bearer| bearer.id == uid)
				.unwrap_or(false);

			if !user_permissions.contains(Permission::UsersGet) { 
				if is_current_user {
					if !user_permissions.contains(Permission::UsersCurrentGet) { 
						let error = PermissionsError::MissingPermission(Permission::UsersCurrentGet);
						return Err(Rejection::from(error));
					}
				} else {
					let error = PermissionsError::MissingPermission(Permission::UsersGet);
					return Err(Rejection::from(error));
				}
			}

			Ok((uid, bearer, connection))
		})
		.untuple_one()
		.then(move |uid: String, _, mut connection: UsersConnection| async move {
			match connection.get_user(&uid).await {
				Ok(user) => {
					warp::reply::json(&user).into_response()
				},
				Err(FetchError::NoItems) => {
					StatusCode::NOT_FOUND.into_response()
				},
				Err(err) => {
					StatusCode::INTERNAL_SERVER_ERROR.into_response()
				},
			}
		})
}

pub fn current(
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("users")
		.and(warp::path("current"))
		.and(warp::path::tail())
		.and(warp::get())
		.and(authorization::authorized(users_db, &[Permission::UsersCurrentGet]))
		.then(|tail: Tail, user: Option<Bearer>, _| async move {
			if let Some(uid) = user.map(|b| b.id) {
				let location = format!("/users/{}/{}", uid, tail.as_str())
					.parse::<Uri>().unwrap();
				warp::redirect::temporary(location).into_response()
			} else {
				StatusCode::UNAUTHORIZED.into_response()
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
	warp::path("users")
		.and(warp::path::param())
		.and(warp::path::end())
		.and(warp::patch())
		.and(warp::body::json())
		.and(authorization::bearer())
		.and(database::connection(users_db))
		.and_then(|uid: String, update, bearer: Option<Bearer>, mut connection: UsersConnection| async {
			let user_permissions = match bearer {
				Some(ref user) => {
					connection.user_permissions(&user.id).await
						.map_err(UsersDBError::Raw)
						.map_err(Rejection::from)?
				}
				None => Permission::defaults(),
			};

			let is_current_user = bearer.as_ref()
				.map(|bearer| bearer.id == uid)
				.unwrap_or(false);

			if !user_permissions.contains(Permission::UsersGet) { 
				if is_current_user {
					if !user_permissions.contains(Permission::UsersCurrentGet) { 
						let error = PermissionsError::MissingPermission(Permission::UsersCurrentGet);
						return Err(Rejection::from(error));
					}
				} else {
					let error = PermissionsError::MissingPermission(Permission::UsersGet);
					return Err(Rejection::from(error));
				}
			}

			if !user_permissions.contains(Permission::UsersPatch) { 
				if is_current_user {
					if !user_permissions.contains(Permission::UsersCurrentPatch) { 
						let error = PermissionsError::MissingPermission(Permission::UsersCurrentPatch);
						return Err(Rejection::from(error));
					}
				} else {
					let error = PermissionsError::MissingPermission(Permission::UsersPatch);
					return Err(Rejection::from(error));
				}
			}

			Ok((uid, update, bearer, connection))
		})
		.untuple_one()
		.then(move |uid: String, update: UserUpdate, _, mut connection: UsersConnection| async move {
			// TODO: validate username

			match connection.update_user(&uid, &update.name).await {
				Ok(()) => {
					match connection.get_user(&uid).await {
						Ok(user) => {
							warp::reply::json(&user).into_response()
						},
						Err(err) => {
							StatusCode::INTERNAL_SERVER_ERROR.into_response()
						},
					}
				},
				Err(UpdateError::NoItem) => {
					StatusCode::NOT_FOUND.into_response()
				},
				Err(err) => {
					StatusCode::INTERNAL_SERVER_ERROR.into_response()
				},
			}
		})
}

pub fn delete(
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("users")
		.and(warp::path::param())
		.and(warp::path::end())
		.and(warp::delete())
		.and(authorization::bearer())
		.and(database::connection(users_db))
		.and_then(|uid: String, bearer: Option<Bearer>, mut connection: UsersConnection| async {
			let user_permissions = match bearer {
				Some(ref user) => {
					connection.user_permissions(&user.id).await
						.map_err(UsersDBError::Raw)
						.map_err(Rejection::from)?
				}
				None => Permission::defaults(),
			};

			let is_current_user = bearer.as_ref()
				.map(|bearer| bearer.id == uid)
				.unwrap_or(false);

			if !user_permissions.contains(Permission::UsersGet) { 
				if is_current_user {
					if !user_permissions.contains(Permission::UsersCurrentGet) { 
						let error = PermissionsError::MissingPermission(Permission::UsersCurrentGet);
						return Err(Rejection::from(error));
					}
				} else {
					let error = PermissionsError::MissingPermission(Permission::UsersGet);
					return Err(Rejection::from(error));
				}
			}

			if !user_permissions.contains(Permission::UsersDelete) { 
				if is_current_user {
					if !user_permissions.contains(Permission::UsersCurrentDelete) { 
						let error = PermissionsError::MissingPermission(Permission::UsersCurrentDelete);
						return Err(Rejection::from(error));
					}
				} else {
					let error = PermissionsError::MissingPermission(Permission::UsersDelete);
					return Err(Rejection::from(error));
				}
			}

			Ok((uid, bearer, connection))
		})
		.untuple_one()
		.then(move |uid: String, _, mut connection: UsersConnection| async move {
			match connection.delete_user(&uid).await {
				Ok(()) => {
					StatusCode::OK.into_response()
				},
				Err(DeleteError::NoItem) => {
					StatusCode::NOT_FOUND.into_response()
				},
				Err(err) => {
					StatusCode::INTERNAL_SERVER_ERROR.into_response()
				},
			}
		})
}

// TODO: patch, delete, and socket stuff 
// (if they are even possible — it seems troublesome with the external stores)