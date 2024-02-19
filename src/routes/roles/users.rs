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

pub fn roles(
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("users")
		.and(warp::path::param())
		.and(warp::path("roles"))
		.and(warp::path::end())
		.and(warp::get())
		.and(authorization::bearer()
			.and_then(with_permission(Permission::UsersGet))
			.and_then(with_permission(Permission::UsersRolesGet)))
		.and(warp::query())
		.and(database::connection(users_db))
		.then(move |uid: String,_user, pagination: PaginationOptions<String>, mut connection: UsersConnection| async move {
			let page = pagination.page;
			let limit = pagination.limit
				.unwrap_or(10)
				.clamp(1, 100);
			
			match connection.list_user_roles(&uid, page, limit).await {
				Ok((page_token, roles)) => {
					let references = roles.iter()
						.map(|r| Reference {
							uri: format!("/roles/{}", r.name).parse().unwrap(),
							view: r,
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