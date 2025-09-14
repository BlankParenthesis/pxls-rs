use std::sync::Arc;

use serde::Deserialize;
use tokio::sync::RwLock;
use warp::http::StatusCode;
use warp::{Filter, Reply, Rejection};

use crate::config::CONFIG;
use crate::filter::resource::filter::FilterRange;
use crate::filter::response::paginated_list::PaginationOptions;
use crate::filter::header::authorization::{self, Bearer};
use crate::permissions::Permission;
use crate::database::{UsersDatabase, UsersConnection, LdapPageToken, UsersDatabaseError, Faction};
use crate::routes::core::{Connections, EventPacket};

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
				.unwrap_or(CONFIG.default_page_item_limit)
				.clamp(1, CONFIG.max_page_item_limit);

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
	events: Arc<RwLock<Connections>>,
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("factions")
		.and(warp::path::end())
		.and(warp::post())
		.and(warp::body::json())
		.and(authorization::authorized(users_db, Permission::FactionsGet | Permission::FactionsPost))
		.then(move |faction: FactionPost, user: Option<Bearer>, mut connection: UsersConnection| {
			let events = Arc::clone(&events);
			async move {
				// FIXME: validate name

				let id = connection.create_faction(&faction.name).await?;

				let mut members = vec![];
				
				let faction = connection.get_faction(&id).await?;

				let member_packet = if let Some(owner) = user {
					let member = connection.add_faction_member(&id, &owner.id, true).await?;
					members.push(owner.id.clone());
					Some(EventPacket::FactionMemberUpdated {
						owners: vec![owner.id.clone()],
						user: owner.id,
						faction: faction.clone(),
						member
					})
				} else {
					None
				};
				
				let events = events.read().await;
				
				let faction_packet = EventPacket::FactionCreated {
					members,
					faction: faction.clone(),
				};
				
				events.send(&faction_packet).await;

				if let Some(packet) = member_packet {
					events.send(&packet).await;
				}

				Ok::<_, UsersDatabaseError>(faction.created())
			}
		})
}

#[derive(Debug, Deserialize)]
struct FactionPatch {
	name: Option<String>,
	icon: Option<url::Url>,
}

pub fn patch(
	events: Arc<RwLock<Connections>>,
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("factions")
		.and(warp::path::param())
		.and(warp::path::end())
		.and(warp::patch())
		.and(warp::body::json())
		.and(authorization::authorized(users_db, Permission::FactionsGet | Permission::FactionsPatch))
		.then(move |id: String, faction: FactionPatch, user: Option<Bearer>, mut connection: UsersConnection| {
			let events = Arc::clone(&events);
			async move {
				// FIXME: validate name
				// FIXME: check if user is owner, possibly add new related permissions
				connection.update_faction(&id, faction.name, faction.icon).await?;

				let faction = connection.get_faction(&id).await?;
				let members = connection.get_all_faction_members(&id).await?;

				let packet = EventPacket::FactionUpdated {
					members,
					faction: faction.clone(),
				};
				let events = events.read().await;
				events.send(&packet).await;

				Ok::<_, UsersDatabaseError>(faction.reply())
			}
		})
}

pub fn delete(
	events: Arc<RwLock<Connections>>,
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("factions")
		.and(warp::path::param())
		.and(warp::path::end())
		.and(warp::delete())
		.and(authorization::authorized(users_db, Permission::FactionsDelete.into()))
		.then(move |id: String, _, mut connection: UsersConnection| {
			let events = Arc::clone(&events);
			async move {
				let members = connection.get_all_faction_members(&id).await?;
				connection.delete_faction(&id).await?;

				let packet = EventPacket::FactionDeleted {
					members,
					faction: Faction::uri(&id),
				};
				let events = events.read().await;
				events.send(&packet).await;

				Ok::<_, UsersDatabaseError>(StatusCode::NO_CONTENT)
			}
		})
}
