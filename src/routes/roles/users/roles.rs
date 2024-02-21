use std::sync::Arc;

use serde::Deserialize;
use warp::{
	http::StatusCode,
	Filter,
	Reply,
	Rejection,
};

use crate::filter::header::authorization::{self, Bearer, UsersDBError, PermissionsError};
use crate::filter::response::paginated_list::{PaginationOptions, Page};
use crate::filter::response::reference::Reference;
use crate::filter::resource::database;

use crate::permissions::Permission;
use crate::database::{UsersDatabase, UsersConnection, FetchError, UpdateError};

pub fn list(
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("users")
		.and(warp::path::param())
		.and(warp::path("roles"))
		.and(warp::path::end())
		.and(warp::get())
		.and(warp::query())
		.and(authorization::bearer())
		.and(database::connection(users_db))
		.and_then(|uid: String, pagination, bearer: Option<Bearer>, mut connection: UsersConnection| async {
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

			if !user_permissions.contains(Permission::UsersRolesGet) { 
				if is_current_user {
					if !user_permissions.contains(Permission::UsersCurrentRolesGet) { 
						let error = PermissionsError::MissingPermission(Permission::UsersCurrentRolesGet);
						return Err(Rejection::from(error));
					}
				} else {
					let error = PermissionsError::MissingPermission(Permission::UsersRolesGet);
					return Err(Rejection::from(error));
				}
			}

			Ok((uid, pagination, bearer, connection))
		})
		.untuple_one()
		.then(move |uid: String, pagination: PaginationOptions<String>, _, mut connection: UsersConnection| async move {
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
						next: page_token.map(|p| {
							format!("/users/{}/roles?limit={}&page={}", uid, limit, p)
						}),
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

#[derive(Deserialize)]
struct RoleSpecifier {
	role: String,
}

pub fn post(
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("users")
		.and(warp::path::param())
		.and(warp::path("roles"))
		.and(warp::path::end())
		.and(warp::post())
		.and(warp::body::json())
		.and(authorization::bearer())
		.and(database::connection(users_db))
		.and_then(|uid: String, role: RoleSpecifier, bearer: Option<Bearer>, mut connection: UsersConnection| async {
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

			if !user_permissions.contains(Permission::UsersRolesPost) { 
				if is_current_user {
					if !user_permissions.contains(Permission::UsersCurrentRolesPost) { 
						let error = PermissionsError::MissingPermission(Permission::UsersCurrentRolesPost);
						return Err(Rejection::from(error));
					}
				} else {
					let error = PermissionsError::MissingPermission(Permission::UsersRolesPost);
					return Err(Rejection::from(error));
				}
			}

			Ok((uid, role, bearer, connection))
		})
		.untuple_one()
		.then(move |uid: String, role: RoleSpecifier, _, mut connection: UsersConnection| async move {
			match connection.add_user_role(&uid, &role.role).await {
				Ok(()) => {
					// TODO: move to a DEFAULT_LIMIT constant somewhere universal
					const LIMIT: usize = 10;
					match connection.list_user_roles(&uid, None, LIMIT).await {
						Ok((page_token, roles)) => {
							let references = roles.iter()
								.map(|r| Reference {
									uri: format!("/roles/{}", r.name).parse().unwrap(),
									view: r,
								})
								.collect::<Vec<_>>();
		
							let page = Page {
								items: &references[..],
								next: page_token.map(|p| {
									format!("/users/{}/roles?limit={}&page={}", uid, LIMIT, p)
								}),
								previous: None, // TODO: previous page
							};
		
							warp::reply::json(&page).into_response()
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
		.and(warp::path("roles"))
		.and(warp::path::end())
		.and(warp::delete())
		.and(warp::body::json())
		.and(authorization::bearer())
		.and(database::connection(users_db))
		.and_then(|uid: String, role: RoleSpecifier, bearer: Option<Bearer>, mut connection: UsersConnection| async {
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

			if !user_permissions.contains(Permission::UsersRolesPost) { 
				if is_current_user {
					if !user_permissions.contains(Permission::UsersCurrentRolesPost) { 
						let error = PermissionsError::MissingPermission(Permission::UsersCurrentRolesPost);
						return Err(Rejection::from(error));
					}
				} else {
					let error = PermissionsError::MissingPermission(Permission::UsersRolesPost);
					return Err(Rejection::from(error));
				}
			}

			Ok((uid, role, bearer, connection))
		})
		.untuple_one()
		.then(move |uid: String, role: RoleSpecifier, _, mut connection: UsersConnection| async move {
			match connection.remove_user_role(&uid, &role.role).await {
				Ok(()) => {
					// TODO: move to a DEFAULT_LIMIT constant somewhere universal
					const LIMIT: usize = 10;
					match connection.list_user_roles(&uid, None, LIMIT).await {
						Ok((page_token, roles)) => {
							let references = roles.iter()
								.map(|r| Reference {
									uri: format!("/roles/{}", r.name).parse().unwrap(),
									view: r,
								})
								.collect::<Vec<_>>();
		
							let page = Page {
								items: &references[..],
								next: page_token.map(|p| {
									format!("/users/{}/roles?limit={}&page={}", uid, LIMIT, p)
								}),
								previous: None, // TODO: previous page
							};
		
							warp::reply::json(&page).into_response()
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