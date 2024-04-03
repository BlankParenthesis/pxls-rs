use std::sync::Arc;

use serde::Deserialize;
use warp::http::{StatusCode, Uri};
use warp::{Filter, Reply, Rejection};

use crate::filter::response::paginated_list::{
	PaginationOptions,
	DEFAULT_PAGE_ITEM_LIMIT,
	MAX_PAGE_ITEM_LIMIT
};
use crate::filter::header::authorization::{self, Bearer};
use crate::filter::response::reference;
use crate::permissions::Permission;
use crate::database::{UsersDatabase, UsersConnection, LdapPageToken};

pub fn list(
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("factions")
		.and(warp::path::param())
		.and(warp::path("members"))
		.and(warp::path::end())
		.and(warp::get())
		.and(warp::query())
		.and(authorization::authorized(users_db, Permission::FactionsMembersList.into()))
		.then(move |fid: String, pagination: PaginationOptions<LdapPageToken>, _, mut connection: UsersConnection| async move {
			let page = pagination.page;
			let limit = pagination.limit
				.unwrap_or(DEFAULT_PAGE_ITEM_LIMIT)
				.clamp(1, MAX_PAGE_ITEM_LIMIT); // TODO: maybe raise upper limit

			connection.list_faction_members(&fid, page, limit).await
				.map(|page| warp::reply::json(&page.into_references()))
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
				.map(|member| warp::reply::json(&member))
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
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("factions")
		.and(warp::path::param())
		.and(warp::path("members"))
		.and(warp::path::end())
		.and(warp::post())
		.and(warp::body::json())
		.and(authorization::authorized(users_db, Permission::FactionsMembersGet | Permission::FactionsMembersPost))
		.then(move |fid: String, member: FactionMemberPost, user: Option<Bearer>, mut connection: UsersConnection| async move {
			// TODO: validate name
			// TODO: validate permissions

			connection.add_faction_member(&fid, &member.user.0, member.owner).await
				.map(|member| reference::created(&member))
		})
}

#[derive(Debug, Deserialize)]
struct FactionMemberPatch {
	// TODO: join intent, update spec
	owner: bool,
}

pub fn patch(
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
		.then(move |fid: String, uid: String, member: FactionMemberPatch, user: Option<Bearer>, mut connection: UsersConnection| async move {
			// TODO: validate name
			// TODO: validate permissions

			connection.edit_faction_member(&fid, &uid, member.owner).await
				.map(|member| warp::reply::json(&member))
		})
}

pub fn delete(
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("factions")
		.and(warp::path::param())
		.and(warp::path("members"))
		.and(warp::path::param())
		.and(warp::path::end())
		.and(warp::delete())
		.and(authorization::authorized(users_db, Permission::FactionsDelete.into()))
		.then(move |fid: String, uid: String, _, mut connection: UsersConnection| async move {
			// TODO: validate name
			// TODO: validate permissions
			connection.remove_faction_member(&fid, &uid).await
				.map(|()| StatusCode::NO_CONTENT)
		})
}
