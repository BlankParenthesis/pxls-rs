use std::sync::Arc;

use serde::Deserialize;
use tokio::sync::RwLock;
use warp::http::StatusCode;
use warp::{Filter, Reply, Rejection};

use crate::config::CONFIG;
use crate::filter::response::paginated_list::PaginationOptions;
use crate::filter::header::authorization;
use crate::filter::response::reference::Reference;
use crate::permissions::Permission;
use crate::database::{Database, DbConn, FactionMemberCurrentSpecifier, FactionMemberListSpecifier, FactionMemberSpecifier, Specifier, User, UserSpecifier};
use crate::routes::core::{Connections, EventPacket};

#[derive(Deserialize, Debug, Default)]
pub struct FactionMemberFilter {
	pub owner: Option<bool>,
	// TODO
	// pub join_intent: JoinIntent,
}

pub fn list(
	db: Arc<Database>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	FactionMemberListSpecifier::path()
		.and(warp::get())
		.and(warp::query())
		.and(warp::query())
		.and(authorization::authorized(db, Permission::FactionsMembersList.into()))
		.then(move |list: FactionMemberListSpecifier, pagination: PaginationOptions<_>, filter: FactionMemberFilter, _, connection: DbConn| async move {
			let page = pagination.page;
			let limit = pagination.limit
				.unwrap_or(CONFIG.default_page_item_limit)
				.clamp(1, CONFIG.max_page_item_limit);

			connection.list_faction_members(&list, page, limit, filter).await
				.map(|page| warp::reply::json(&page))
		})
}

pub fn get(
	db: Arc<Database>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	FactionMemberSpecifier::path()
		.and(warp::get())
		.and(authorization::authorized(db, Permission::FactionsMembersGet.into()))
		.then(move |member: FactionMemberSpecifier, _, connection: DbConn| async move {
			connection.get_faction_member(&member).await?
				.ok_or(StatusCode::NOT_FOUND)
				.map(|member| warp::reply::json(&member))
		})
}

pub fn current(
	boards_db: Arc<Database>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	FactionMemberCurrentSpecifier::path()
		.and(warp::get())
		.and(authorization::authorized(boards_db, Permission::FactionsMembersCurrentGet.into()))
		.then(move |member: FactionMemberCurrentSpecifier, user: Option<User>, connection: DbConn| async move {
			if let Some(user) = user {
				let member = member.member(user.specifier());
				connection.get_faction_member(&member).await?
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
	db: Arc<Database>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	FactionMemberListSpecifier::path()
		.and(warp::post())
		.and(warp::body::json())
		.and(authorization::authorized(db, Permission::FactionsMembersGet | Permission::FactionsMembersPost))
		.then(move |list: FactionMemberListSpecifier, mut post: FactionMemberPost, user: Option<User>, connection: DbConn| {
			let events = Arc::clone(&events);
			async move {
				let is_current_user = user.map(|u| *u.specifier() == post.user)
					.unwrap_or(false);
				// FIXME: validate permissions  more
				post.owner = false;
				if !is_current_user {
					return Err(StatusCode::FORBIDDEN);
				}
				let member_specifier = post.user;
				
				// TODO: event update for size change
				// NOTE: maybe bundle these as with place events since a lot of them
				// could happen in a given time frame (to reduce network load)

				let member = connection.create_faction_member(
					&list,
					&member_specifier,
					post.owner,
					true,
					true,
				).await?;
				
				let owners = connection.all_faction_owners(&list).await?;
				let faction = Reference::from(member.faction().clone());
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
	db: Arc<Database>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	FactionMemberSpecifier::path()
		.and(warp::patch())
		.and(warp::body::json())
		.and(authorization::authorized(db, Permission::FactionsMembersGet | Permission::FactionsMembersPatch))
		.then(move |member: FactionMemberSpecifier, patch: FactionMemberPatch, requester: Option<User>, connection: DbConn| {
			let events = Arc::clone(&events);
			async move {
				// FIXME: validate permissions

				let update = connection.update_faction_member(
					&member,
					Some(patch.owner),
					None,
					None,
				).await?
				.ok_or(StatusCode::NOT_FOUND)?;

				let owners = connection.all_faction_owners(&member.list()).await?;
				let faction = Reference::from(update.faction().clone());
				let reference = Reference::from(update);

				let packet = EventPacket::FactionMemberUpdated {
					owners,
					user: member.user(),
					faction,
					member: reference.clone(),
				};
				let events = events.read().await;
				events.send(&packet).await;

				Ok::<_, StatusCode>(reference.reply())
			}
		})
}

pub fn delete(
	events: Arc<RwLock<Connections>>,
	db: Arc<Database>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	FactionMemberSpecifier::path()
		.and(warp::delete())
		.and(authorization::authorized(db, Permission::FactionsMembersDelete.into()))
		.then(move |member: FactionMemberSpecifier, requester: Option<User>, connection: DbConn| {
			let events = Arc::clone(&events);
			async move {
				let is_current_user = requester.map(|u| *u.specifier() == member.user())
					.unwrap_or(false);
				
				// FIXME: validate permissions more
				if !is_current_user {
					return Err(StatusCode::FORBIDDEN);
				}
				
				// TODO: event update for size change
				// NOTE: maybe bundle these as with place events since a lot of them
				// could happen in a given time frame (to reduce network load)

				let delete = connection.delete_faction_member(&member).await?
					.ok_or(StatusCode::NOT_FOUND)?;

				let owners = connection.all_faction_owners(&member.list()).await?;
				let faction = Reference::from(delete.faction().clone());
				let reference = Reference::from(delete);

				let packet = EventPacket::FactionMemberUpdated {
					owners,
					faction,
					member: reference,
					user: member.user(),
				};
				let events = events.read().await;
				events.send(&packet).await;

				Ok(StatusCode::NO_CONTENT)
			}
		})
}
