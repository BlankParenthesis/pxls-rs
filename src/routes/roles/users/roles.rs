use std::sync::Arc;

use serde::Deserialize;
use warp::{
	Filter,
	Reply,
	Rejection,
};

use crate::filter::header::authorization::{self, Bearer, PermissionsError};
use crate::filter::response::paginated_list::{PaginationOptions, Page, MAX_PAGE_ITEM_LIMIT, DEFAULT_PAGE_ITEM_LIMIT};
use crate::filter::response::reference::Reference;

use crate::permissions::Permission;
use crate::database::{UsersDatabase, UsersConnection};

pub fn list(
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	let permissions = Permission::UsersGet | Permission::UsersRolesGet;

	warp::path("users")
		.and(warp::path::param())
		.and(warp::path("roles"))
		.and(warp::path::end())
		.and(warp::get())
		.and(warp::query())
		.and(authorization::permissions(users_db))
		.and_then(move |uid: String, pagination, user_permissions, bearer: Option<Bearer>, connection| async move {
			let is_current_user = bearer.as_ref()
				.map(|bearer| bearer.id == uid)
				.unwrap_or(false);

			let check = if is_current_user {
				authorization::has_permissions_current
			} else {
				authorization::has_permissions
			};


			if check(user_permissions, permissions) {
				Ok((uid, pagination, connection))
			} else {
				Err(warp::reject::custom(PermissionsError::MissingPermission))
			}
		})
		.untuple_one()
		.and_then(move |uid: String, pagination: PaginationOptions<String>, mut connection: UsersConnection| async move {
			let page = pagination.page;
			let limit = pagination.limit
				.unwrap_or(DEFAULT_PAGE_ITEM_LIMIT)
				.clamp(1, MAX_PAGE_ITEM_LIMIT);
			
			connection.list_user_roles(&uid, page, limit).await
				.map(|(page_token, roles)| {
					let references = roles.iter()
						.map(Reference::from)
						.collect::<Vec<_>>();

					let page = Page {
						items: &references[..],
						next: page_token.map(|p| {
							format!("/users/{}/roles?limit={}&page={}", uid, limit, p)
						}),
						// TODO: either find some magical way to generate this or
						// change the spec
						previous: None,
					};

					warp::reply::json(&page).into_response()
				})
				.map_err(warp::reject::custom)
		})
}

#[derive(Deserialize)]
struct RoleSpecifier {
	role: String,
}

pub fn post(
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	let permissions = Permission::UsersGet | Permission::UsersRolesPost;

	warp::path("users")
		.and(warp::path::param())
		.and(warp::path("roles"))
		.and(warp::path::end())
		.and(warp::post())
		.and(warp::body::json())
		.and(authorization::permissions(users_db))
		.and_then(move |uid: String, role, user_permissions, bearer: Option<Bearer>, connection| async move {
			let is_current_user = bearer.as_ref()
				.map(|bearer| bearer.id == uid)
				.unwrap_or(false);

			let check = if is_current_user {
				authorization::has_permissions_current
			} else {
				authorization::has_permissions
			};

			if check(user_permissions, permissions) {
				Ok((uid, role, connection))
			} else {
				Err(warp::reject::custom(PermissionsError::MissingPermission))
			}
		})
		.untuple_one()
		.then(move |uid: String, role: RoleSpecifier, mut connection: UsersConnection| async move {
			connection.add_user_role(&uid, &role.role).await?;
			connection.list_user_roles(&uid, None, DEFAULT_PAGE_ITEM_LIMIT).await
				.map(|(page_token, roles)| {
					let references = roles.iter()
						.map(Reference::from)
						.collect::<Vec<_>>();

					let page = Page {
						items: &references[..],
						next: page_token.map(|p| {
							format!("/users/{}/roles?limit={}&page={}", uid, DEFAULT_PAGE_ITEM_LIMIT, p)
						}),
						previous: None, // TODO: previous page
					};

					warp::reply::json(&page)
				})
		})
}

pub fn delete(
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	let permissions = Permission::UsersGet | Permission::UsersRolesDelete;

	warp::path("users")
		.and(warp::path::param())
		.and(warp::path("roles"))
		.and(warp::path::end())
		.and(warp::delete())
		.and(warp::body::json())
		.and(authorization::permissions(users_db))
		.and_then(move |uid: String, role, user_permissions, bearer: Option<Bearer>, connection| async move {
			let is_current_user = bearer.as_ref()
				.map(|bearer| bearer.id == uid)
				.unwrap_or(false);

			let check = if is_current_user {
				authorization::has_permissions_current
			} else {
				authorization::has_permissions
			};


			if check(user_permissions, permissions) {
				Ok((uid, role, connection))
			} else {
				Err(warp::reject::custom(PermissionsError::MissingPermission))
			}
		})
		.untuple_one()
		.then(move |uid: String, role: RoleSpecifier, mut connection: UsersConnection| async move {
			connection.remove_user_role(&uid, &role.role).await?;
			connection.list_user_roles(&uid, None, DEFAULT_PAGE_ITEM_LIMIT).await
				.map(|(page_token, roles)| {
					let references = roles.iter()
						.map(Reference::from)
						.collect::<Vec<_>>();

					let page = Page {
						items: &references[..],
						next: page_token.map(|p| {
							format!("/users/{}/roles?limit={}&page={}", uid, DEFAULT_PAGE_ITEM_LIMIT, p)
						}),
						previous: None, // TODO: previous page
					};

					warp::reply::json(&page)
				})
		})
}