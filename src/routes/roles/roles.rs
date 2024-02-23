use std::sync::Arc;

use reqwest::header;
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
use crate::database::{UsersDatabase, UsersConnection, Role};

pub fn list(
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("roles")
		.and(warp::path::end())
		.and(warp::get())
		.and(warp::query())
		.and(authorized(users_db, Permission::RolesList.into()))
		.and_then(move |pagination: PaginationOptions<String>, _, mut connection: UsersConnection| async move {
			let page = pagination.page;
			let limit = pagination.limit
				.unwrap_or(DEFAULT_PAGE_ITEM_LIMIT)
				.clamp(1, MAX_PAGE_ITEM_LIMIT);
			
			connection.list_roles(page, limit).await
				.map(|(page_token, roles)| {
					let references = roles.iter()
						.map(Reference::from)
						.collect::<Vec<_>>();

					let page = Page {
						items: &references[..],
						next: page_token.map(|p| format!("/roles?limit={}&page={}", limit, p)),
						// TODO: either find some magical way to generate this or
						// change the spec
						previous: None,
					};

					warp::reply::json(&page)
				})
				.map_err(warp::reject::custom)
		})
}


pub fn get(
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("roles")
		.and(warp::path::param())
		.and(warp::path::end())
		.and(warp::get())
		.and(authorized(users_db, Permission::RolesGet.into()))
		.and_then(move |role: String, _, mut connection: UsersConnection| async move {
			connection.get_role(&role).await
				.map(|role| warp::reply::json(&role))
				.map_err(warp::reject::custom)
		})
}

pub fn post(
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("roles")
		.and(warp::path::end())
		.and(warp::post())
		.and(warp::body::json())
		.and(authorized(users_db, Permission::RolesPost.into()))
		.and_then(move |role: Role, _, mut connection: UsersConnection| async move {
			connection.create_role(&role).await?;
			connection.get_role(&role.name).await
				.map(|role| {
					let reference = &Reference::from(&role);
					let response = warp::reply::with_status(
						warp::reply::json(reference),
						StatusCode::CREATED
					);
					warp::reply::with_header(
						response,
						header::LOCATION,
						reference.uri.to_string(),
					)
				})
				.map_err(warp::reject::custom)
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
		.and(authorized(users_db, Permission::RolesPatch.into()))
		.and_then(move |role: String, new_role: RoleUpdate, _, mut connection: UsersConnection| async move {
			connection.update_role(
				role.as_str(),
				new_role.name.as_deref(),
				new_role.icon,
				new_role.permissions,
			).await?;
			connection.get_role(&role).await
				.map(|role| warp::reply::json(&role))
				.map_err(warp::reject::custom)
		})
}

pub fn delete(
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("roles")
		.and(warp::path::param())
		.and(warp::path::end())
		.and(warp::delete())
		.and(authorized(users_db, Permission::RolesDelete.into()))
		.and_then(move |role: String, _, mut connection: UsersConnection| async move {
			connection.delete_role(&role).await
				.map(|()| StatusCode::OK) 
				.map_err(warp::reject::custom)
		})
}