use std::sync::Arc;

use serde::Deserialize;
use tokio::sync::RwLock;
use warp::http::{StatusCode, Uri};
use warp::{Filter, Reply, Rejection};

use crate::filter::response::paginated_list::{
	PaginationOptions,
	DEFAULT_PAGE_ITEM_LIMIT,
	MAX_PAGE_ITEM_LIMIT
};
use crate::filter::header::authorization::{self, Bearer};
use crate::filter::response::reference::Reference;
use crate::permissions::Permission;
use crate::database::{UsersDatabase, UsersConnection, LdapPageToken, UsersDatabaseError, FactionMember, User, JoinIntent};
use crate::routes::core::{Connections, EventPacket};

#[derive(Deserialize, Debug, Default)]
pub struct MemberFilter {
	pub owner: Option<bool>,
	// TODO
	// pub join_intent: JoinIntent,
}

pub fn list(
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("factions")
		.and(warp::path::param())
		.and(warp::path("members"))
		.and(warp::path::end())
		.and(warp::get())
		.and(warp::query())
		.and(warp::query())
		.and(authorization::authorized(users_db, Permission::FactionsMembersList.into()))
		.then(move |fid: String, pagination: PaginationOptions<LdapPageToken>, filter: MemberFilter, _, mut connection: UsersConnection| async move {
			let page = pagination.page;
			let limit = pagination.limit
				.unwrap_or(DEFAULT_PAGE_ITEM_LIMIT)
				.clamp(1, MAX_PAGE_ITEM_LIMIT); // TODO: maybe raise upper limit

			connection.list_faction_members(&fid, page, limit, filter).await
				.map(|page| warp::reply::json(&page))
		})
}

pub fn get(
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("factions")
		.and(warp::path::param())
		.and(warp::path("members"))
		.and(warp::path::param())
		.and(warp::path::end())
		.and(warp::get())
		.and(authorization::authorized(users_db, Permission::FactionsMembersGet.into()))
		.then(move |fid: String, uid: String, _, mut connection: UsersConnection| async move {
			connection.get_faction_member(&fid, &uid).await
				.map(|member| member.deref())
		})
}

pub fn current(
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("factions")
		.and(warp::path::param())
		.and(warp::path("members"))
		.and(warp::path("current"))
		.and(warp::path::end())
		.and(warp::get())
		.and(authorization::authorized(users_db, Permission::FactionsMembersCurrentGet.into()))
		.then(move |fid: String, user: Option<Bearer>, mut connection: UsersConnection| async move {
			if let Some(uid) = user.map(|b| b.id) {
				connection.get_faction_member(&fid, &uid).await
					.map(|member| member.reply())
					.map_err(StatusCode::from)
			} else {
				Err(StatusCode::UNAUTHORIZED)
			}
		})
}

#[derive(Debug)]
struct UserSpecifier(String);

// TODO: this is probably handy to have for other types and in other places too
impl<'de> Deserialize<'de> for UserSpecifier {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where D: serde::Deserializer<'de> {
		struct V;

		impl<'de> serde::de::Visitor<'de> for V {
			type Value = UserSpecifier;
			
			fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
				write!(f, "A user uri reference")
			}

			fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
			where E: serde::de::Error, {
				let uri = v.parse::<Uri>().map_err(E::custom)?;
				// TODO: maybe domain/scheme validation
				let mut segments = uri.path().split('/');

				if !matches!(segments.next(), Some("")) {
					return Err(E::custom("expected absolute path"))
				}
				
				if !matches!(segments.next(), Some("users")) {
					return Err(E::custom("expected /users/"))
				}
				
				let id = match segments.next() {
					Some(id) => id,
					None => return Err(E::custom("expected user id")),
				};
				
				if let Some(unexpected) = segments.next() {
					let error = format!("unexpected path segment \"{}\"", unexpected);
					return Err(E::custom(error));
				}

				Ok(UserSpecifier(id.to_string()))
			}
		}

		deserializer.deserialize_str(V)
	}
}

#[derive(Debug, Deserialize)]
struct FactionMemberPost {
	user: UserSpecifier,
	// TODO: join intent, update spec
	owner: bool,
}

pub fn post(
	events: Arc<RwLock<Connections>>,
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("factions")
		.and(warp::path::param())
		.and(warp::path("members"))
		.and(warp::path::end())
		.and(warp::post())
		.and(warp::body::json())
		.and(authorization::authorized(users_db, Permission::FactionsMembersGet | Permission::FactionsMembersPost))
		.then(move |fid: String, member: FactionMemberPost, user: Option<Bearer>, mut connection: UsersConnection| {
			let events = Arc::clone(&events);
			async move {
				// FIXME: validate permissions

				let uid = member.user.0;
				
				// TODO: event update for size change
				// NOTE: maybe bundle these as with place events since a lot of them
				// could happen in a given time frame (to reduce network load)

				let member = connection.add_faction_member(
					&fid,
					&uid,
					member.owner,
				).await?;
				
				let faction = connection.get_faction(&fid).await?;
				let owners = connection.get_faction_owners(&fid).await?;

				let packet = EventPacket::FactionMemberUpdated {
					owners,
					user: uid,
					faction,
					member: member.clone(),
				};
				let events = events.read().await;
				events.send(&packet).await;

				Ok::<_, UsersDatabaseError>(member.created())
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
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("factions")
		.and(warp::path::param())
		.and(warp::path("members"))
		.and(warp::path::param())
		.and(warp::path::end())
		.and(warp::patch())
		.and(warp::body::json())
		.and(authorization::authorized(users_db, Permission::FactionsMembersGet | Permission::FactionsMembersPatch))
		.then(move |fid: String, uid: String, member: FactionMemberPatch, user: Option<Bearer>, mut connection: UsersConnection|  {
			let events = Arc::clone(&events);
			async move {
				// FIXME: validate permissions

				let member = connection.edit_faction_member(
					&fid,
					&uid,
					member.owner,
				).await?;


				let faction = connection.get_faction(&fid).await?;
				let owners = connection.get_faction_owners(&fid).await?;

				let packet = EventPacket::FactionMemberUpdated {
					owners,
					user: uid,
					faction,
					member: member.clone(),
				};
				let events = events.read().await;
				events.send(&packet).await;

				Ok::<_, UsersDatabaseError>(member.reply())
			}
		})
}

pub fn delete(
	events: Arc<RwLock<Connections>>,
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("factions")
		.and(warp::path::param())
		.and(warp::path("members"))
		.and(warp::path::param())
		.and(warp::path::end())
		.and(warp::delete())
		.and(authorization::authorized(users_db, Permission::FactionsMembersDelete.into()))
		.then(move |fid: String, uid: String, user: Option<Bearer>, mut connection: UsersConnection| {
			let events = Arc::clone(&events);
			async move {
				// FIXME: validate permissions
				
				// TODO: event update for size change
				// NOTE: maybe bundle these as with place events since a lot of them
				// could happen in a given time frame (to reduce network load)

				connection.remove_faction_member(&fid, &uid).await?;

				let faction = connection.get_faction(&fid).await?;
				let owners = connection.get_faction_owners(&fid).await?;
				let user = connection.get_user(&uid).await?;
				let member = FactionMember {
					owner: false,
					join_intent: JoinIntent {
						member: false,
						faction: false,
					},
					user: Reference::new(User::uri(&uid), user),
				};

				let packet = EventPacket::FactionMemberUpdated {
					owners,
					faction,
					member: Reference::new(FactionMember::uri(&fid, &uid), member),
					user: uid,
				};
				let events = events.read().await;
				events.send(&packet).await;

				Ok::<_, UsersDatabaseError>(StatusCode::NO_CONTENT)
			}
		})
}
