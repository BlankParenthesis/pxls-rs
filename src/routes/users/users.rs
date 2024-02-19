use std::sync::Arc;

use warp::{
	http::{StatusCode, Uri},
	Filter,
	Reply,
	Rejection,
	path::Tail,
};

use crate::filter::response::paginated_list::{PaginationOptions, Page};
use crate::filter::response::reference::Reference;
use crate::filter::header::authorization::{Bearer, authorized};
use crate::permissions::Permission;
use crate::database::{UsersDatabase, UsersConnection, FetchError};

pub fn list(
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("users")
		.and(warp::path::end())
		.and(warp::get())
		.and(warp::query())
		.and(authorized(users_db, &[Permission::UsersList]))
		.then(move |pagination: PaginationOptions<String>, _, mut connection: UsersConnection| async move {
			let page = pagination.page;
			let limit = pagination.limit
				.unwrap_or(10)
				.clamp(1, 100); // TODO: maybe raise upper limit
			
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
					// TODO: find all these and just to status.into_response()
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
		.and(authorized(users_db, &[Permission::UsersGet]))
		.then(move |uid: String, _, mut connection: UsersConnection| async move {
			match connection.get_user(&uid).await {
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

pub fn current(
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("users")
		.and(warp::path("current"))
		.and(warp::path::tail())
		.and(warp::get())
		// TODO: current user permissions
		.and(authorized(users_db, &[Permission::UsersList]))
		.then(|tail: Tail, user: Option<Bearer>, _| async move {
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