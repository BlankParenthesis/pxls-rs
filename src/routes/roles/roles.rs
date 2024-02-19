use std::sync::Arc;

use warp::{
	http::StatusCode,
	Filter,
	Reply,
	Rejection,
};

use crate::filter::header::authorization::{self, with_permission};
use crate::filter::response::paginated_list::{PaginationOptions, Page};
use crate::filter::response::reference::Reference;
use crate::filter::resource::database;

use crate::permissions::Permission;
use crate::database::{UsersDatabase, UsersConnection, FetchError};

pub fn list(
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("roles")
		.and(warp::path::end())
		.and(warp::get())
		.and(authorization::bearer().and_then(with_permission(Permission::RolesList)))
		.and(warp::query())
		.and(database::connection(users_db))
		.then(move |_user, pagination: PaginationOptions<String>, mut connection: UsersConnection| async move {
			let page = pagination.page;
			let limit = pagination.limit
				.unwrap_or(10)
				.clamp(1, 100);
			
			match connection.list_roles(page, limit).await {
				Ok((page_token, roles)) => {
					let references = roles.iter()
						.map(|r| Reference {
							uri: format!("/roles/{}", r.name).parse().unwrap(),
							view: r,
						})
						.collect::<Vec<_>>();

					let page = Page {
						items: &references[..],
						next: page_token.map(|p| format!("/roles?limit={}&page={}", limit, p)),
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
					eprintln!("{:?}", err);
					StatusCode::INTERNAL_SERVER_ERROR.into_response()
				},
			}
		})
}


pub fn get(
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("roles")
		.and(warp::path::param())
		.and(warp::path::end())
		.and(warp::get())
		// TODO: current user permissions
		.and(authorization::bearer().and_then(with_permission(Permission::RolesGet)))
		.and(database::connection(users_db))
		.then(move |role: String, _user, mut connection: UsersConnection| async move {
			match connection.get_role(&role).await {
				Ok(role) => {
					warp::reply::json(&role).into_response()
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