use std::sync::Arc;

use serde::Deserialize;
use tokio::sync::RwLock;
use url::Url;
use warp::{
	http::StatusCode,
	Filter,
	Reply,
	Rejection,
};

use crate::{filter::response::{paginated_list::{
	PaginationOptions,
	DEFAULT_PAGE_ITEM_LIMIT,
	MAX_PAGE_ITEM_LIMIT
}, reference::Reference}, routes::core::Connections, database::UsersDatabaseError};
use crate::filter::response::reference;
use crate::filter::header::authorization::authorized;
use crate::permissions::Permission;
use crate::database::{UsersDatabase, UsersConnection, Role, LdapPageToken};
use crate::routes::core::EventPacket;

pub fn list(
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("roles")
		.and(warp::path::end())
		.and(warp::get())
		.and(warp::query())
		.and(authorized(users_db, Permission::RolesList.into()))
		.then(move |pagination: PaginationOptions<LdapPageToken>, _, mut connection: UsersConnection| async move {
			let page = pagination.page;
			let limit = pagination.limit
				.unwrap_or(DEFAULT_PAGE_ITEM_LIMIT)
				.clamp(1, MAX_PAGE_ITEM_LIMIT);
			
			connection.list_roles(page, limit).await
				.map(|page| warp::reply::json(&page.into_references()))
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
		.then(move |role: String, _, mut connection: UsersConnection| async move {
			connection.get_role(&role).await
				.map(|role| warp::reply::json(&role))
		})
}

pub fn post(
	events_sockets: Arc<RwLock<Connections>>,
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("roles")
		.and(warp::path::end())
		.and(warp::post())
		.and(warp::body::json())
		.and(authorized(users_db, Permission::RolesPost.into()))
		.then(move |role: Role, _, mut connection: UsersConnection| {
			let events_sockets = events_sockets.clone();
			async move {
				connection.create_role(&role).await?;
				let role = connection.get_role(&role.name).await?;
				
				let packet = EventPacket::RoleCreated {
					role: Reference::from(&role),
				};
				events_sockets.read().await.send(&packet);

				Ok::<_, UsersDatabaseError>(reference::created(&role))
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

// TODO: for this and all other reasonable patches: require if-not-modified precondition
pub fn patch(
	events_sockets: Arc<RwLock<Connections>>,
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("roles")
		.and(warp::path::param())
		.and(warp::path::end())
		.and(warp::patch())
		.and(warp::body::json())
		.and(authorized(users_db, Permission::RolesPatch.into()))
		.then(move |role: String, new_role: RoleUpdate, _, mut connection: UsersConnection| {
			let events_sockets = events_sockets.clone();
			async move {
				connection.update_role(
					role.as_str(),
					new_role.name.as_deref(),
					new_role.icon,
					new_role.permissions,
				).await?;
				
				let role = connection.get_role(&role).await?;

				let packet = EventPacket::RoleUpdated {
					role: Reference::from(&role),
				};
				events_sockets.read().await.send(&packet).await;
				
				Ok::<_, UsersDatabaseError>(warp::reply::json(&role))
			}
		})
}

pub fn delete(
	events_sockets: Arc<RwLock<Connections>>,
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("roles")
		.and(warp::path::param())
		.and(warp::path::end())
		.and(warp::delete())
		.and(authorized(users_db, Permission::RolesDelete.into()))
		.then(move |role: String, _, mut connection: UsersConnection| {
			let events_sockets = events_sockets.clone();
			async move {
				let response = connection.delete_role(&role).await
					.map(|()| StatusCode::NO_CONTENT);
			
				let packet = EventPacket::RoleDeleted {
					role: format!("/roles/{}", role).parse().unwrap(),
				};
				events_sockets.read().await.send(&packet).await;

				response
			}
		})
}