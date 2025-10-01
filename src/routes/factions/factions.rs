use std::sync::Arc;

use serde::Deserialize;
use tokio::sync::RwLock;
use warp::http::StatusCode;
use warp::{Filter, Reply, Rejection};

use crate::config::CONFIG;
use crate::filter::resource::filter::FilterRange;
use crate::filter::response::paginated_list::PaginationOptions;
use crate::filter::header::authorization;
use crate::filter::response::reference::Reference;
use crate::permissions::Permission;
use crate::database::{Database, DbConn, FactionListSpecifier, FactionSpecifier, Specifier, User};
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
	db: Arc<Database>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	FactionListSpecifier::path()
		.and(warp::query())
		.and(warp::query())
		.and(authorization::authorized(db, Permission::FactionsList.into()))
		.then(move |_, pagination: PaginationOptions<_>, filter: FactionFilter, _, connection: DbConn| async move {
			let page = pagination.page;
			let limit = pagination.limit
				.unwrap_or(CONFIG.default_page_item_limit)
				.clamp(1, CONFIG.max_page_item_limit);

			connection.list_factions(page, limit, filter).await
				.map(|page| warp::reply::json(&page))
		})
}

pub fn get(
	db: Arc<Database>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	FactionSpecifier::path()
		.and(authorization::authorized(db, Permission::FactionsGet.into()))
		.then(move |faction: FactionSpecifier, _, connection: DbConn| async move {
			connection.get_faction(&faction).await
				.map(|faction| warp::reply::json(&faction))
		})
}

#[derive(Debug, Deserialize)]
struct FactionPost {
	name: String,
	icon: Option<url::Url>,
}

pub fn post(
	events: Arc<RwLock<Connections>>,
	db: Arc<Database>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	FactionListSpecifier::path()
		.and(warp::post())
		.and(warp::body::json())
		.and(authorization::authorized(db, Permission::FactionsGet | Permission::FactionsPost))
		.then(move |_, faction: FactionPost, user: Option<User>, connection: DbConn| {
			let events = Arc::clone(&events);
			async move {
				// FIXME: validate name

				let faction = connection.create_faction(faction.name, faction.icon).await?;

				let member_list = faction.specifier().members();
				let mut members = vec![];
				let faction = Reference::from(faction);
				
				let member_packet = if let Some(owner) = user {
					let owner_specifier = *owner.specifier();
					let owner = connection.create_faction_member(
						&member_list,
						&owner_specifier,
						true,
						true,
						true,
					).await?;
					
					members.push(owner_specifier);
					Some(EventPacket::FactionMemberUpdated {
						owners: vec![owner_specifier],
						user: owner_specifier,
						faction: faction.clone(),
						member: Reference::from(owner)
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

				Ok::<_, StatusCode>(faction.created())
			}
		})
}

#[derive(Debug, Deserialize)]
struct FactionPatch {
	name: Option<String>,
	#[serde(default, with = "serde_with::rust::double_option")]
	icon: Option<Option<url::Url>>,
}

pub fn patch(
	events: Arc<RwLock<Connections>>,
	db: Arc<Database>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	FactionSpecifier::path()
		.and(warp::patch())
		.and(warp::body::json())
		.and(authorization::authorized(db, Permission::FactionsGet | Permission::FactionsPatch))
		.then(move |faction: FactionSpecifier, patch: FactionPatch, user: Option<User>, connection: DbConn| {
			let events = Arc::clone(&events);
			async move {
				// FIXME: validate name
				// FIXME: check if user is owner, possibly add new related permissions
				let update = connection.update_faction(&faction, patch.name, patch.icon).await?
					.ok_or(StatusCode::NOT_FOUND)?;
				let members = connection.all_faction_members(&faction.members()).await?;
				let faction = Reference::from(update);

				let packet = EventPacket::FactionUpdated {
					members,
					faction: faction.clone(),
				};
				let events = events.read().await;
				events.send(&packet).await;

				Ok::<_, StatusCode>(faction.reply())
			}
		})
}

pub fn delete(
	events: Arc<RwLock<Connections>>,
	db: Arc<Database>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	FactionSpecifier::path()
		.and(warp::delete())
		.and(authorization::authorized(db, Permission::FactionsDelete.into()))
		.then(move |faction: FactionSpecifier, _, connection: DbConn| {
			let events = Arc::clone(&events);
			async move {
				let members = connection.all_faction_members(&faction.members()).await?;
				connection.delete_faction(&faction).await?;

				let packet = EventPacket::FactionDeleted {
					members,
					faction,
				};
				let events = events.read().await;
				events.send(&packet).await;

				Ok::<_, StatusCode>(StatusCode::NO_CONTENT)
			}
		})
}
