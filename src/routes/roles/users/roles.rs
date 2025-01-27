use std::sync::Arc;

use serde::Deserialize;
use tokio::sync::RwLock;
use warp::{
	Filter,
	Reply,
	Rejection,
};

use crate::config::CONFIG;
use crate::filter::header::authorization::{self, Bearer, PermissionsError};
use crate::filter::response::paginated_list::{
	PaginationOptions,
	PageToken,
};

use crate::permissions::Permission;
use crate::database::{UsersDatabase, UsersConnection, LdapPageToken, UsersDatabaseError};
use crate::routes::core::{Connections, EventPacket};
use crate::routes::roles::roles::RoleFilter;

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
		.and(warp::query())
		.and(authorization::permissions(users_db))
		.and_then(move |uid: String, pagination, filter, user_permissions, bearer: Option<Bearer>, connection| async move {
			let is_current_user = bearer.as_ref()
				.map(|bearer| bearer.id == uid)
				.unwrap_or(false);

			let check = if is_current_user {
				authorization::has_permissions_current
			} else {
				authorization::has_permissions
			};


			if check(user_permissions, permissions) {
				Ok((uid, pagination, filter, connection))
			} else {
				Err(warp::reject::custom(PermissionsError::MissingPermission))
			}
		})
		.untuple_one()
		.then(move |uid: String, pagination: PaginationOptions<LdapPageToken>, filter: RoleFilter, mut connection: UsersConnection| async move {
			let page = pagination.page;
			let limit = pagination.limit
				.unwrap_or(CONFIG.default_page_item_limit)
				.clamp(1, CONFIG.max_page_item_limit);
			
			connection.list_user_roles(&uid, page, limit, filter).await
				.map(|page| warp::reply::json(&page))
		})
}

#[derive(Deserialize)]
struct RoleSpecifier {
	role: String,
}

pub fn post(
	event_sockets: Arc<RwLock<Connections>>,
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
		.then(move |uid: String, role: RoleSpecifier, mut connection: UsersConnection|{
			let event_sockets = event_sockets.clone();
			async move {
				let prior = connection.user_permissions(Some(uid.clone())).await?;
				connection.add_user_role(&uid, &role.role).await?;
				let after = connection.user_permissions(Some(uid.clone())).await?;
				let roles = connection.list_user_roles(
					&uid,
					PageToken::start(),
					CONFIG.default_page_item_limit,
					RoleFilter::default(),
				).await?;

				let connections = event_sockets.read().await;
				if prior != after {
					let packet = EventPacket::AccessUpdate {
						user_id: Some(uid.clone()),
						permissions: after,
					};
					connections.send(&packet).await;
				}
				
				let packet = EventPacket::UserRolesUpdated {
					user: format!("/users/{}", uid).parse().unwrap(),
					user_id: Some(uid),
				};
				connections.send(&packet).await;

				Ok::<_, UsersDatabaseError>(warp::reply::json(&roles))
			}
		})
}

pub fn delete(
	event_sockets: Arc<RwLock<Connections>>,
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
		.then(move |uid: String, role: RoleSpecifier, mut connection: UsersConnection| {
			let event_sockets = event_sockets.clone();
			async move {
				let prior = connection.user_permissions(Some(uid.clone())).await?;
				connection.remove_user_role(&uid, &role.role).await?;
				let after = connection.user_permissions(Some(uid.clone())).await?;
				let roles = connection.list_user_roles(
					&uid,
					PageToken::start(),
					CONFIG.default_page_item_limit,
					RoleFilter::default(),
				).await?;

				let connections = event_sockets.read().await;
				if prior != after {
					let packet = EventPacket::AccessUpdate {
						permissions: after,
						user_id: Some(uid.clone()),
					};
					connections.send(&packet).await;
				}

				let packet = EventPacket::UserRolesUpdated {
					user: format!("/users/{}", uid).parse().unwrap(),
					user_id: Some(uid),
				};
				connections.send(&packet).await;
				
				Ok::<_, UsersDatabaseError>(warp::reply::json(&roles))
			}
		})
}
