use std::sync::Arc;

use sea_orm::TryInsertResult;
use serde::Deserialize;
use tokio::sync::RwLock;
use warp::http::StatusCode;
use warp::{Filter, Reply, Rejection};

use crate::config::CONFIG;
use crate::filter::response::paginated_list::PaginationOptions;
use crate::filter::header::authorization;
use crate::filter::response::reference::Reference;
use crate::permissions::Permission;
use crate::database::{BoardsConnection, BoardsDatabase, FactionSpecifier, User, UserSpecifier};
use crate::routes::core::{Connections, EventPacket};

#[derive(Deserialize, Debug, Default)]
pub struct FactionMemberFilter {
	pub owner: Option<bool>,
	// TODO
	// pub join_intent: JoinIntent,
}

pub fn list(
	db: Arc<BoardsDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("factions")
		.and(warp::path::param())
		.and(warp::path("members"))
		.and(warp::path::end())
		.and(warp::get())
		.and(warp::query())
		.and(warp::query())
		.and(authorization::authorized(db, Permission::FactionsMembersList.into()))
		.then(move |faction: FactionSpecifier, pagination: PaginationOptions<_>, filter: FactionMemberFilter, _, connection: BoardsConnection| async move {
			let page = pagination.page;
			let limit = pagination.limit
				.unwrap_or(CONFIG.default_page_item_limit)
				.clamp(1, CONFIG.max_page_item_limit);

			connection.list_faction_members(&faction, page, limit, filter).await
				.map(|page| warp::reply::json(&page))
		})
}

pub fn get(
	db: Arc<BoardsDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("factions")
		.and(warp::path::param())
		.and(warp::path("members"))
		.and(warp::path::param())
		.and(warp::path::end())
		.and(warp::get())
		.and(authorization::authorized(db, Permission::FactionsMembersGet.into()))
		.then(move |faction: FactionSpecifier, user: UserSpecifier, _, connection: BoardsConnection| async move {
			connection.get_faction_member(&faction, &user).await?
				.ok_or(StatusCode::NOT_FOUND)
				.map(|member| warp::reply::json(&member))
		})
}

pub fn current(
	boards_db: Arc<BoardsDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("factions")
		.and(warp::path::param())
		.and(warp::path("members"))
		.and(warp::path("current"))
		.and(warp::path::end())
		.and(warp::get())
		.and(authorization::authorized(boards_db, Permission::FactionsMembersCurrentGet.into()))
		.then(move |faction: FactionSpecifier, user: Option<User>, connection: BoardsConnection| async move {
			if let Some(user) = user {
				connection.get_faction_member(&faction, &user.specifier()).await?
					.ok_or(StatusCode::NOT_FOUND)
					.map(Reference::from)
					.map(|r| r.reply())
			} else {
				Err(StatusCode::UNAUTHORIZED)
			}
		})
}

#[derive(Debug, Deserialize)]
struct FactionMemberPost {
	user: UserSpecifier,
	// TODO: join intent, update spec
	owner: bool,
}

pub fn post(
	events: Arc<RwLock<Connections>>,
	db: Arc<BoardsDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("factions")
		.and(warp::path::param())
		.and(warp::path("members"))
		.and(warp::path::end())
		.and(warp::post())
		.and(warp::body::json())
		.and(authorization::authorized(db, Permission::FactionsMembersGet | Permission::FactionsMembersPost))
		.then(move |faction: FactionSpecifier, mut member: FactionMemberPost, user: Option<User>, connection: BoardsConnection| {
			let events = Arc::clone(&events);
			async move {
				let is_current_user = user.map(|u| u.specifier() == member.user)
					.unwrap_or(false);
				// FIXME: validate permissions  more
				member.owner = false;
				if !is_current_user {
					return Err(StatusCode::FORBIDDEN);
				}
				let member_specifier = member.user;
				
				// TODO: event update for size change
				// NOTE: maybe bundle these as with place events since a lot of them
				// could happen in a given time frame (to reduce network load)

				let member = connection.create_faction_member(
					&faction,
					&member_specifier,
					member.owner,
					true,
					true,
				).await?;
				
				let member = match member {
					TryInsertResult::Inserted(member) => member,
					TryInsertResult::Conflicted => return Err(StatusCode::CONFLICT),
					TryInsertResult::Empty => return Err(StatusCode::NOT_FOUND),
				};
				
				let owners = connection.all_faction_owners(&faction).await?;
				let faction = member.faction();
				let member = Reference::from(member);

				let packet = EventPacket::FactionMemberUpdated {
					owners,
					user: member_specifier,
					faction,
					member: member.clone(),
				};
				let events = events.read().await;
				events.send(&packet).await;

				Ok(member.created())
			}
		})
}

#[derive(Debug, Deserialize)]
struct FactionMemberPatch {
	// TODO: join intent, update spec
	owner: bool,
}

pub fn patch(
	events: Arc<RwLock<Connections>>,
	db: Arc<BoardsDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("factions")
		.and(warp::path::param())
		.and(warp::path("members"))
		.and(warp::path::param())
		.and(warp::path::end())
		.and(warp::patch())
		.and(warp::body::json())
		.and(authorization::authorized(db, Permission::FactionsMembersGet | Permission::FactionsMembersPatch))
		.then(move |faction: FactionSpecifier, user: UserSpecifier, patch: FactionMemberPatch, requester: Option<User>, connection: BoardsConnection| {
			let events = Arc::clone(&events);
			async move {
				// FIXME: validate permissions

				let member = connection.update_faction_member(
					&faction,
					&user,
					Some(patch.owner),
					None,
					None,
				).await?
				.ok_or(StatusCode::NOT_FOUND)?;

				let owners = connection.all_faction_owners(&faction).await?;
				let faction = member.faction();
				let member = Reference::from(member);

				let packet = EventPacket::FactionMemberUpdated {
					owners,
					user,
					faction,
					member: member.clone(),
				};
				let events = events.read().await;
				events.send(&packet).await;

				Ok::<_, StatusCode>(member.reply())
			}
		})
}

pub fn delete(
	events: Arc<RwLock<Connections>>,
	db: Arc<BoardsDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("factions")
		.and(warp::path::param())
		.and(warp::path("members"))
		.and(warp::path::param())
		.and(warp::path::end())
		.and(warp::delete())
		.and(authorization::authorized(db, Permission::FactionsMembersDelete.into()))
		.then(move |faction: FactionSpecifier, user: UserSpecifier, requester: Option<User>, connection: BoardsConnection| {
			let events = Arc::clone(&events);
			async move {
				let is_current_user = requester.map(|u| u.specifier() == user)
					.unwrap_or(false);
				
				// FIXME: validate permissions more
				if !is_current_user {
					return Err(StatusCode::FORBIDDEN);
				}
				
				// TODO: event update for size change
				// NOTE: maybe bundle these as with place events since a lot of them
				// could happen in a given time frame (to reduce network load)

				let member = connection.delete_faction_member(&faction, &user).await?
					.ok_or(StatusCode::NOT_FOUND)?;

				let owners = connection.all_faction_owners(&faction).await?;
				let faction = member.faction();
				let member = Reference::from(member);

				let packet = EventPacket::FactionMemberUpdated {
					owners,
					faction,
					member,
					user,
				};
				let events = events.read().await;
				events.send(&packet).await;

				Ok(StatusCode::NO_CONTENT)
			}
		})
}
