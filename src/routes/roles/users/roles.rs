use std::sync::Arc;

use reqwest::StatusCode;
use sea_orm::TryInsertResult;
use serde::Deserialize;
use tokio::sync::RwLock;
use warp::{Filter, Reply, Rejection};

use crate::config::CONFIG;
use crate::filter::header::authorization::{self, PermissionsError};
use crate::filter::response::paginated_list::{PaginationOptions, PageToken};

use crate::permissions::Permission;
use crate::database::{BoardsConnection, BoardsDatabase, RoleSpecifier, User, UserSpecifier};
use crate::routes::core::{Connections, EventPacket};
use crate::routes::roles::roles::RoleFilter;

pub fn list(
	db: Arc<BoardsDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	let permissions = Permission::UsersGet | Permission::UsersRolesGet;

	warp::path("users")
		.and(warp::path::param())
		.and(warp::path("roles"))
		.and(warp::path::end())
		.and(warp::get())
		.and(warp::query())
		.and(warp::query())
		.and(authorization::permissions(db))
		.and_then(move |user: UserSpecifier, pagination, filter, user_permissions, requester: Option<User>, connection| async move {
			let is_current_user = requester.as_ref()
				.map(|u| u.specifier() == user)
				.unwrap_or(false);

			let check = if is_current_user {
				authorization::has_permissions_current
			} else {
				authorization::has_permissions
			};


			if check(user_permissions, permissions) {
				Ok((user, pagination, filter, connection))
			} else {
				Err(warp::reject::custom(PermissionsError::MissingPermission))
			}
		})
		.untuple_one()
		.then(move |user: UserSpecifier, pagination: PaginationOptions<_>, filter: RoleFilter, connection: BoardsConnection| async move {
			let page = pagination.page;
			let limit = pagination.limit
				.unwrap_or(CONFIG.default_page_item_limit)
				.clamp(1, CONFIG.max_page_item_limit);
			
			connection.list_user_roles(&user, page, limit, filter).await
				.map(|page| warp::reply::json(&page))
		})
}

#[derive(Deserialize)]
struct NewRoleMember {
	role: RoleSpecifier,
}

pub fn post(
	event_sockets: Arc<RwLock<Connections>>,
	db: Arc<BoardsDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	let permissions = Permission::UsersGet | Permission::UsersRolesPost;

	warp::path("users")
		.and(warp::path::param())
		.and(warp::path("roles"))
		.and(warp::path::end())
		.and(warp::post())
		.and(warp::body::json())
		.and(authorization::permissions(db))
		.and_then(move |user: UserSpecifier, role, user_permissions, requester: Option<User>, connection| async move {
			let is_current_user = requester.as_ref()
				.map(|u| u.specifier() == user)
				.unwrap_or(false);

			let check = if is_current_user {
				authorization::has_permissions_current
			} else {
				authorization::has_permissions
			};

			if check(user_permissions, permissions) {
				Ok((user, role, connection))
			} else {
				Err(warp::reject::custom(PermissionsError::MissingPermission))
			}
		})
		.untuple_one()
		.then(move |user: UserSpecifier, role: NewRoleMember, connection: BoardsConnection|{
			let event_sockets = event_sockets.clone();
			async move {
				let prior = connection.user_permissions(&user).await?;
				match connection.create_role_member(&user, &role.role).await? {
					TryInsertResult::Inserted(_) => (),
					TryInsertResult::Conflicted => return Err(StatusCode::CONFLICT),
					TryInsertResult::Empty => return Err(StatusCode::NOT_FOUND),
				}
				let after = connection.user_permissions(&user).await?;
				let roles = connection.list_user_roles(
					&user,
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
				
				let packet = EventPacket::UserRolesUpdated {
					user: format!("/users/{}", user).parse().unwrap(),
					specifier: Some(user),
				};
				connections.send(&packet).await;

				Ok::<_, StatusCode>(warp::reply::json(&roles))
			}
		})
}

pub fn delete(
	event_sockets: Arc<RwLock<Connections>>,
	db: Arc<BoardsDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	let permissions = Permission::UsersGet | Permission::UsersRolesDelete;

	warp::path("users")
		.and(warp::path::param())
		.and(warp::path("roles"))
		.and(warp::path::end())
		.and(warp::delete())
		.and(warp::body::json())
		.and(authorization::permissions(db))
		.and_then(move |user: UserSpecifier, role, user_permissions, requester: Option<User>, connection| async move {
			let is_current_user = requester.as_ref()
				.map(|u| u.specifier() == user)
				.unwrap_or(false);

			let check = if is_current_user {
				authorization::has_permissions_current
			} else {
				authorization::has_permissions
			};


			if check(user_permissions, permissions) {
				Ok((user, role, connection))
			} else {
				Err(warp::reject::custom(PermissionsError::MissingPermission))
			}
		})
		.untuple_one()
		.then(move |user: UserSpecifier, role: NewRoleMember, connection: BoardsConnection| {
			let event_sockets = event_sockets.clone();
			async move {
				let prior = connection.user_permissions(&user).await?;
				connection.delete_role_member(&user, &role.role).await?
					.ok_or(StatusCode::NOT_FOUND)?;
				
				let after = connection.user_permissions(&user).await?;
				let roles = connection.list_user_roles(
					&user,
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

				let packet = EventPacket::UserRolesUpdated {
					user: format!("/users/{}", user).parse().unwrap(),
					specifier: Some(user),
				};
				connections.send(&packet).await;
				
				Ok::<_, StatusCode>(warp::reply::json(&roles))
			}
		})
}
