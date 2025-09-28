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

use crate::config::CONFIG;
use crate::filter::response::paginated_list::PaginationOptions;
use crate::routes::core::Connections;
use crate::filter::header::authorization::authorized;
use crate::permissions::Permission;
use crate::database::{BoardsConnection, BoardsDatabase, RoleSpecifier};
use crate::routes::core::EventPacket;

#[derive(Deserialize, Debug, Default)]
pub struct RoleFilter {
	pub name: Option<String>,
	pub icon: Option<String>, // TODO: handle explicit null?
	// TODO: array stuff as mentioned elsewhere
	// pub permissions: Vec<Permission>,
}

pub fn list(
	db: Arc<BoardsDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("roles")
		.and(warp::path::end())
		.and(warp::get())
		.and(warp::query())
		.and(warp::query())
		.and(authorized(db, Permission::RolesList.into()))
		.then(move |pagination: PaginationOptions<_>, filter: RoleFilter, _, connection: BoardsConnection| async move {
			let page = pagination.page;
			let limit = pagination.limit
				.unwrap_or(CONFIG.default_page_item_limit)
				.clamp(1, CONFIG.max_page_item_limit);
			
			connection.list_roles(page, limit, filter).await
				.map(|page| warp::reply::json(&page))
		})
}


pub fn get(
	db: Arc<BoardsDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("roles")
		.and(warp::path::param())
		.and(warp::path::end())
		.and(warp::get())
		.and(authorized(db, Permission::RolesGet.into()))
		.then(move |role: RoleSpecifier, _, connection: BoardsConnection| async move {
			connection.get_role(&role).await
				.map(|role| warp::reply::json(&role))
		})
}

#[derive(Deserialize)]
struct NewRole {
	name: String,
	#[serde(default)]
	icon: Option<Url>,
	permissions: Vec<Permission>,
}

pub fn post(
	events_sockets: Arc<RwLock<Connections>>,
	db: Arc<BoardsDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("roles")
		.and(warp::path::end())
		.and(warp::post())
		.and(warp::body::json())
		.and(authorized(db, Permission::RolesPost.into()))
		.then(move |role: NewRole, _, connection: BoardsConnection| {
			let events_sockets = events_sockets.clone();
			async move {
				let role = connection.create_role(
					role.name,
					role.icon,
					role.permissions.into_iter().collect(),
				).await?;
				
				let packet = EventPacket::RoleCreated {
					role: role.clone(),
				};
				events_sockets.read().await.send(&packet).await;

				Ok::<_, StatusCode>(role.created()) // TODO: not sure if created is correct
			}
		})
}

#[derive(Deserialize)]
struct RoleUpdate {
	name: Option<String>,
	#[serde(default, with = "serde_with::rust::double_option")]
	icon: Option<Option<Url>>,
	permissions: Option<Vec<Permission>>,
}

// TODO: for this and all other reasonable patches: require if-not-modified precondition
pub fn patch(
	events_sockets: Arc<RwLock<Connections>>,
	db: Arc<BoardsDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("roles")
		.and(warp::path::param())
		.and(warp::path::end())
		.and(warp::patch())
		.and(warp::body::json())
		.and(authorized(db, Permission::RolesPatch.into()))
		.then(move |role: RoleSpecifier, new_role: RoleUpdate, _, connection: BoardsConnection| {
			let events_sockets = events_sockets.clone();
			async move {
				let role = connection.update_role(
					&role,
					new_role.name,
					new_role.icon,
					new_role.permissions.map(|p| p.into_iter().collect()),
				).await?
				.ok_or(StatusCode::NOT_FOUND)?;
				
				let packet = EventPacket::RoleUpdated {
					role: role.clone(),
				};
				events_sockets.read().await.send(&packet).await;
				
				Ok::<_, StatusCode>(warp::reply::json(&role))
			}
		})
}

pub fn delete(
	events_sockets: Arc<RwLock<Connections>>,
	db: Arc<BoardsDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("roles")
		.and(warp::path::param())
		.and(warp::path::end())
		.and(warp::delete())
		.and(authorized(db, Permission::RolesDelete.into()))
		.then(move |role: RoleSpecifier, _, connection: BoardsConnection| {
			let events_sockets = events_sockets.clone();
			async move {
				let response = connection.delete_role(&role).await
					.map(|_| StatusCode::NO_CONTENT);
			
				let packet = EventPacket::RoleDeleted {
					role: format!("/roles/{}", role).parse().unwrap(),
				};
				events_sockets.read().await.send(&packet).await;

				response
			}
		})
}
