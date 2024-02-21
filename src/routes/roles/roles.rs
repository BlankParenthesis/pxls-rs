use std::sync::Arc;

use serde::Deserialize;
use url::Url;
use warp::{
	http::StatusCode,
	Filter,
	Reply,
	Rejection,
};

use crate::filter::response::paginated_list::{PaginationOptions, Page, DEFAULT_PAGE_ITEM_LIMIT, MAX_PAGE_ITEM_LIMIT};
use crate::filter::response::reference::Reference;
use crate::filter::header::authorization::authorized;
use crate::permissions::Permission;
use crate::database::{UsersDatabase, UsersConnection, FetchError, Role, DeleteError, UpdateError, CreateError};

pub fn list(
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("roles")
		.and(warp::path::end())
		.and(warp::get())
		.and(warp::query())
		.and(authorized(users_db, &[Permission::RolesList]))
		.then(move |pagination: PaginationOptions<String>, _, mut connection: UsersConnection| async move {
			let page = pagination.page;
			let limit = pagination.limit
				.unwrap_or(DEFAULT_PAGE_ITEM_LIMIT)
				.clamp(1, MAX_PAGE_ITEM_LIMIT);
			
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
		.and(authorized(users_db, &[Permission::RolesGet]))
		.then(move |role: String, _, mut connection: UsersConnection| async move {
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

pub fn post(
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("roles")
		.and(warp::path::end())
		.and(warp::post())
		.and(warp::body::json())
		.and(authorized(users_db, &[Permission::RolesPost]))
		.then(move |role: Role, _, mut connection: UsersConnection| async move {
			match connection.create_role(&role).await {
				Ok(()) => {
					match connection.get_role(&role.name).await {
						Ok(role) => {
							let reference = Reference {
								uri: format!("/roles/{}", role.name).parse().unwrap(),
								view: &role,
							};
							warp::reply::json(&reference).into_response()
						},
						Err(err) => {
							StatusCode::INTERNAL_SERVER_ERROR.into_response()
						},
					}
				},
				Err(CreateError::AlreadyExists) => {
					StatusCode::CONFLICT.into_response()
				},
				Err(err) => {
					eprintln!("{:?}", err);
					StatusCode::INTERNAL_SERVER_ERROR.into_response()
				},
			}
		})
}

#[derive(Deserialize)]
struct RoleUpdate {
	name: Option<String>,
	#[serde(with = "serde_with::rust::double_option")]
	icon: Option<Option<Url>>,
	permissions: Option<Vec<Permission>>,
}

pub fn patch(
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("roles")
		.and(warp::path::param())
		.and(warp::path::end())
		.and(warp::patch())
		.and(warp::body::json())
		.and(authorized(users_db, &[Permission::RolesPatch]))
		.then(move |role: String, new_role: RoleUpdate, _, mut connection: UsersConnection| async move {
			let update = connection.update_role(
				role.as_str(),
				new_role.name.as_deref(),
				new_role.icon,
				new_role.permissions,
			);
			match update.await {
				Ok(()) => {
					match connection.get_role(&role).await {
						Ok(role) => {
							warp::reply::json(&role).into_response()
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
					eprintln!("{:?}", err);
					StatusCode::INTERNAL_SERVER_ERROR.into_response()
				},
			}
		})
}

pub fn delete(
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("roles")
		.and(warp::path::param())
		.and(warp::path::end())
		.and(warp::delete())
		.and(authorized(users_db, &[Permission::RolesDelete]))
		.then(move |role: String, _, mut connection: UsersConnection| async move {
			match connection.delete_role(&role).await {
				Ok(role) => {
					StatusCode::OK.into_response()
				},
				Err(DeleteError::NoItem) => {
					StatusCode::NOT_FOUND.into_response()
				},
				Err(err) => {
					eprintln!("{:?}", err);
					StatusCode::INTERNAL_SERVER_ERROR.into_response()
				},
			}
		})
}