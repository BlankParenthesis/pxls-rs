use std::sync::Arc;

use http::{StatusCode, Uri};
use warp::{Filter, Reply, Rejection, path::Tail};

use crate::{
	access::permissions::{with_permission, Permission},
	filters::{
		header::{
			authorization,
		},
		resource::users,
	},
	objects::{
		paginated_list::{PaginationOptions, Page},
		reference::Reference,
		user::AuthedUser
	},
	users::{Pool, Connection, UserFetchError},
};

pub fn list(pool: &Arc<Pool>) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("users")
		.and(warp::path::end())
		.and(warp::get())
		.and(authorization::bearer().and_then(with_permission(Permission::UsersList)))
		.and(warp::query())
		.and(users::connection(pool))
		.then(move |_user, pagination: PaginationOptions<String>, mut connection: Connection| async move {
			let page = pagination.page;
			let limit = pagination.limit
				.unwrap_or(10)
				.clamp(1, 100); // TODO: maybe raise upper limit
			
			let users = crate::users::list(&mut connection, page, limit).await;
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
						previous: None,
					};

					warp::reply::json(&page).into_response()
				},
				Err(UserFetchError::InvalidPage) => {
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

pub fn get(pool: &Arc<Pool>) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("users")
		.and(warp::path::param())
		.and(warp::path::end())
		.and(warp::get())
		// TODO: current user permissions
		.and(authorization::bearer().and_then(with_permission(Permission::UsersGet)))
		.and(users::connection(pool))
		.then(move |uid: String, _user, mut connection: Connection| async move {
			let users = crate::users::get(&mut connection, uid).await;

			match users {
				Ok(user) => {
					warp::reply::json(&user).into_response()
				},
				Err(UserFetchError::MissingUser) => {
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
		.then(|tail: Tail, user: AuthedUser| async move {
			if let Some(uid) = user.user().and_then(|u| u.id.as_ref()) {
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