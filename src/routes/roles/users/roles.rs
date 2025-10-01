use std::sync::Arc;

use reqwest::StatusCode;
use serde::Deserialize;
use tokio::sync::RwLock;
use warp::{Filter, Reply, Rejection};

use crate::config::CONFIG;
use crate::filter::header::authorization::{self, PermissionsError};
use crate::filter::response::paginated_list::{PaginationOptions, PageToken};

use crate::permissions::Permission;
use crate::database::{Database, DbConn, RoleSpecifier, Specifier, User, UserRolesListSpecifier};
use crate::routes::core::{Connections, EventPacket};
use crate::routes::roles::roles::RoleFilter;

pub fn list(
	db: Arc<Database>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	let permissions = Permission::UsersGet | Permission::UsersRolesGet;

	UserRolesListSpecifier::path()
		.and(warp::get())
		.and(warp::query())
		.and(warp::query())
		.and(authorization::permissions(db))
		.and_then(move |list: UserRolesListSpecifier, pagination, filter, user_permissions, requester: Option<User>, connection| async move {
			let is_current_user = requester.as_ref()
				.map(|u| *u.specifier() == list.user())
				.unwrap_or(false);

			let check = if is_current_user {
				authorization::has_permissions_current
			} else {
				authorization::has_permissions
			};

			if check(user_permissions, permissions) {
				Ok((list, pagination, filter, connection))
			} else {
				Err(warp::reject::custom(PermissionsError::MissingPermission))
			}
		})
		.untuple_one()
		.then(move |list: UserRolesListSpecifier, pagination: PaginationOptions<_>, filter: RoleFilter, connection: DbConn| async move {
			let page = pagination.page;
			let limit = pagination.limit
				.unwrap_or(CONFIG.default_page_item_limit)
				.clamp(1, CONFIG.max_page_item_limit);
			
			connection.list_user_roles(&list, page, limit, filter).await
				.map(|page| warp::reply::json(&page))
		})
}

#[derive(Deserialize)]
struct NewRoleMember {
	role: RoleSpecifier,
}

pub fn post(
	event_sockets: Arc<RwLock<Connections>>,
	db: Arc<Database>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	let permissions = Permission::UsersGet | Permission::UsersRolesPost;

	UserRolesListSpecifier::path()
		.and(warp::post())
		.and(warp::body::json())
		.and(authorization::permissions(db))
		.and_then(move |list: UserRolesListSpecifier, role, user_permissions, requester: Option<User>, connection| async move {
			let is_current_user = requester.as_ref()
				.map(|u| *u.specifier() == list.user())
				.unwrap_or(false);

			let check = if is_current_user {
				authorization::has_permissions_current
			} else {
				authorization::has_permissions
			};

			if check(user_permissions, permissions) {
				Ok((list, role, connection))
			} else {
				Err(warp::reject::custom(PermissionsError::MissingPermission))
			}
		})
		.untuple_one()
		.then(move |list: UserRolesListSpecifier, role: NewRoleMember, connection: DbConn|{
			let event_sockets = event_sockets.clone();
			async move {
				let user = list.user();
				let prior = connection.user_permissions(&user).await?;
				let _member = connection.create_role_member(&user, &role.role).await?;
				let after = connection.user_permissions(&user).await?;
				let roles = connection.list_user_roles(
					&list,
					PageToken::start(),
					CONFIG.default_page_item_limit,
					RoleFilter::default(),
				).await?;

				let connections = event_sockets.read().await;
				if prior != after {
					let packet = EventPacket::AccessUpdate {
						user: Some(user),
						permissions: after,
					};
					connections.send(&packet).await;
				}
				
				let packet = EventPacket::UserRolesUpdated { user };
				connections.send(&packet).await;

				Ok::<_, StatusCode>(warp::reply::json(&roles))
			}
		})
}

pub fn delete(
	event_sockets: Arc<RwLock<Connections>>,
	db: Arc<Database>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	let permissions = Permission::UsersGet | Permission::UsersRolesDelete;

	UserRolesListSpecifier::path()
		.and(warp::delete())
		.and(warp::body::json())
		.and(authorization::permissions(db))
		.and_then(move |list: UserRolesListSpecifier, role, user_permissions, requester: Option<User>, connection| async move {
			let is_current_user = requester.as_ref()
				.map(|u| *u.specifier() == list.user())
				.unwrap_or(false);

			let check = if is_current_user {
				authorization::has_permissions_current
			} else {
				authorization::has_permissions
			};


			if check(user_permissions, permissions) {
				Ok((list, role, connection))
			} else {
				Err(warp::reject::custom(PermissionsError::MissingPermission))
			}
		})
		.untuple_one()
		.then(move |list: UserRolesListSpecifier, role: NewRoleMember, connection: DbConn| {
			let event_sockets = event_sockets.clone();
			async move {
				let user = list.user();
				let prior = connection.user_permissions(&user).await?;

				if !connection.delete_role_member(&user, &role.role).await? {
					return Err(StatusCode::NOT_FOUND);
				}
				
				let after = connection.user_permissions(&user).await?;
				let roles = connection.list_user_roles(
					&list,
					PageToken::start(),
					CONFIG.default_page_item_limit,
					RoleFilter::default(),
				).await?;

				let connections = event_sockets.read().await;
				if prior != after {
					let packet = EventPacket::AccessUpdate {
						permissions: after,
						user: Some(user),
					};
					connections.send(&packet).await;
				}

				let packet = EventPacket::UserRolesUpdated { user };
				connections.send(&packet).await;
				
				Ok::<_, StatusCode>(warp::reply::json(&roles))
			}
		})
}
