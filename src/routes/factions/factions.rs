use std::sync::Arc;

use serde::Deserialize;
use warp::http::StatusCode;
use warp::{Filter, Reply, Rejection};

use crate::filter::resource::filter::FilterRange;
use crate::filter::response::paginated_list::{
	PaginationOptions,
	DEFAULT_PAGE_ITEM_LIMIT,
	MAX_PAGE_ITEM_LIMIT
};
use crate::filter::header::authorization::{self, Bearer};
use crate::permissions::Permission;
use crate::database::{UsersDatabase, UsersConnection, LdapPageToken};

pub mod members;

#[derive(Deserialize, Debug, Default)]
pub struct FactionFilter {
	pub name: Option<String>,
	#[serde(default)]
	pub created_at: FilterRange<i64>,
	#[serde(default)]
	pub size: FilterRange<usize>,
}

pub fn list(
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("factions")
		.and(warp::path::end())
		.and(warp::get())
		.and(warp::query())
		.and(warp::query())
		.and(authorization::authorized(users_db, Permission::FactionsList.into()))
		.then(move |pagination: PaginationOptions<LdapPageToken>, filter: FactionFilter, _, mut connection: UsersConnection| async move {
			let page = pagination.page;
			let limit = pagination.limit
				.unwrap_or(DEFAULT_PAGE_ITEM_LIMIT)
				.clamp(1, MAX_PAGE_ITEM_LIMIT); // TODO: maybe raise upper limit

			connection.list_factions(page, limit, filter).await
				.map(|page| warp::reply::json(&page))
		})
}

pub fn get(
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("factions")
		.and(warp::path::param())
		.and(warp::path::end())
		.and(warp::get())
		.and(authorization::authorized(users_db, Permission::FactionsGet.into()))
		.then(move |id: String, _, mut connection: UsersConnection| async move {
			connection.get_faction(&id).await
				.map(|faction| warp::reply::json(&faction))
		})
}

#[derive(Debug, Deserialize)]
struct FactionPost {
	name: String,
}

pub fn post(
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("factions")
		.and(warp::path::end())
		.and(warp::post())
		.and(warp::body::json())
		.and(authorization::authorized(users_db, Permission::FactionsGet | Permission::FactionsPost))
		.then(move |faction: FactionPost, user: Option<Bearer>, mut connection: UsersConnection| async move {
			// FIXME: validate name

			let id = connection.create_faction(&faction.name).await?;

			if let Some(owner) = user {
				connection.add_faction_member(&id, &owner.id, true).await?;
			}

			connection.get_faction(&id).await
				.map(|faction| faction.created())
		})
}

#[derive(Debug, Deserialize)]
struct FactionPatch {
	name: String,
}

pub fn patch(
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("factions")
		.and(warp::path::param())
		.and(warp::path::end())
		.and(warp::patch())
		.and(warp::body::json())
		.and(authorization::authorized(users_db, Permission::FactionsGet | Permission::FactionsPatch))
		.then(move |id: String, faction: FactionPatch, user: Option<Bearer>, mut connection: UsersConnection| async move {
			// FIXME: validate name
			// FIXME: check if user is owner, possibly add new related permissions
			connection.update_faction(&id, &faction.name).await?;

			connection.get_faction(&id).await
				.map(|faction| warp::reply::json(&faction))
		})
}

pub fn delete(
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("factions")
		.and(warp::path::param())
		.and(warp::path::end())
		.and(warp::delete())
		.and(authorization::authorized(users_db, Permission::FactionsDelete.into()))
		.then(move |id: String, _, mut connection: UsersConnection| async move {
			connection.delete_faction(&id).await
				.map(|()| StatusCode::NO_CONTENT)
		})
}

