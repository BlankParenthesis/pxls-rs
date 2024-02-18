use std::sync::Arc;

use warp::{
	http::{StatusCode, Uri},
	Filter,
	Reply,
	Rejection,
	path::Tail,
};

use crate::filter::header::authorization::{self, with_permission, Bearer};
use crate::filter::response::paginated_list::{PaginationOptions, Page};
use crate::filter::response::reference::Reference;
use crate::filter::resource::database;

use crate::permissions::Permission;
use crate::database::{UsersDatabase, UsersConnection, FetchError};

pub fn list(
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("users")
		.and(warp::path::end())
		.and(warp::get())
		.and(authorization::bearer().and_then(with_permission(Permission::UsersList)))
		.and(warp::query())
		.and(database::connection(users_db))
		.then(move |_user, pagination: PaginationOptions<String>, mut connection: UsersConnection| async move {
			let page = pagination.page;
			let limit = pagination.limit
				.unwrap_or(10)
				.clamp(1, 100); // TODO: maybe raise upper limit
			
			let users = connection.list_users(page, limit).await;
			match users {
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
					warp::reply::with_status(
						"",
						StatusCode::BAD_REQUEST
					).into_response()
				},
				Err(err) => {
					warp::reply::with_status(
						"",
						StatusCode::INTERNAL_SERVER_ERROR
					).into_response()
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
		// TODO: current user permissions
		.and(authorization::bearer().and_then(with_permission(Permission::UsersGet)))
		.and(database::connection(users_db))
		.then(move |uid: String, _user, mut connection: UsersConnection| async move {
			let users = connection.get_user(&uid).await;

			match users {
				Ok(user) => {
					warp::reply::json(&user).into_response()
				},
				Err(FetchError::NoItems) => {
					warp::reply::with_status(
						"",
						StatusCode::NOT_FOUND
					).into_response()
				},
				Err(err) => {
					warp::reply::with_status(
						"",
						StatusCode::INTERNAL_SERVER_ERROR
					).into_response()
				},
			}
		})
}

pub fn current() -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("users")
		.and(warp::path("current"))
		.and(warp::path::tail())
		.and(warp::get())
		// TODO: current user permissions
		.and(authorization::bearer().and_then(with_permission(Permission::UsersGet)))
		.then(|tail: Tail, user: Option<Bearer>| async move {
			if let Some(uid) = user.map(|b| b.id) {
				let location = format!("/users/{}/{}", uid, tail.as_str())
					.parse::<Uri>().unwrap();
				warp::redirect::temporary(location).into_response()
			} else {
				warp::reply::with_status(
					"",
					StatusCode::UNAUTHORIZED
				).into_response()
			}
		})
}

// TODO: patch, delete, and socket stuff 
// (if they are even possible — it seems troublesome with the external stores)