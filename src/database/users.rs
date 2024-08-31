use std::{collections::{HashSet, HashMap}, fmt};

use base64::prelude::*;
use deadpool::managed::PoolError;
use enumset::EnumSet;
use ldap3::{
	LdapError,
	controls::{PagedResults, Control, ControlType},
	Scope,
	SearchEntry,
	ldap_escape, Mod,
};

mod connection;
mod entities;

pub use entities::{User, Role, Faction, FactionMember, JoinIntent, ParseError};
use connection::Connection as LdapConnection;
use connection::LDAPConnectionManager;
use connection::Pool;
use reqwest::StatusCode;
use serde::de::{Deserialize, Visitor};
use tokio::sync::RwLock;
use url::{Url, form_urlencoded::byte_serialize};
use warp::{reject::Reject, reply::Reply};

use crate::{config::CONFIG, filter::response::reference::Reference};
use crate::routes::roles::roles::RoleFilter;
use crate::routes::factions::factions::{FactionFilter, members::MemberFilter};
use crate::permissions::Permission;
use crate::filter::response::paginated_list::{Page, PageToken};

use self::entities::LDAPTimestamp;

#[derive(Debug, Default)]
pub struct LdapPageToken(Vec<u8>);

impl PageToken for LdapPageToken {
	fn start() -> Self { Self::default() }
}

impl fmt::Display for LdapPageToken {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{}", BASE64_URL_SAFE.encode(&self.0))
	} 
}

struct Base64Visitor;

impl<'de> Visitor<'de> for Base64Visitor {
	type Value = Vec<u8>;
	
	fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
		write!(formatter, "A base 64 string")
	}

	fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
	where E: serde::de::Error {
		BASE64_URL_SAFE.decode(v).map_err(E::custom)
	}
}

impl<'de> Deserialize<'de> for LdapPageToken {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where D: serde::Deserializer<'de> {
		deserializer.deserialize_str(Base64Visitor).map(LdapPageToken)
	}
}

#[derive(Debug)]
pub enum UsersDatabaseError {
	Fetch(FetchError),
	Create(CreateError),
	Update(UpdateError),
	Delete(DeleteError),
}

impl Reject for UsersDatabaseError {}
impl From<&UsersDatabaseError> for StatusCode {
	fn from(error: &UsersDatabaseError) -> Self {
		match error {
			UsersDatabaseError::Update(UpdateError::NoItem) |
			UsersDatabaseError::Delete(DeleteError::NoItem) |
			UsersDatabaseError::Fetch(FetchError::NoItems) => {
				StatusCode::NOT_FOUND
			},
			UsersDatabaseError::Fetch(FetchError::InvalidPage) => {
				StatusCode::BAD_REQUEST
			},
			UsersDatabaseError::Create(CreateError::AlreadyExists) => {
				StatusCode::CONFLICT
			},
			UsersDatabaseError::Fetch(FetchError::MissingPagerData) |
			UsersDatabaseError::Fetch(FetchError::AmbiguousItems) |
			UsersDatabaseError::Fetch(FetchError::ParseError(_)) |
			UsersDatabaseError::Fetch(FetchError::LdapError(_)) |
			UsersDatabaseError::Fetch(FetchError::IntegrityError) |
			UsersDatabaseError::Create(CreateError::LdapError(_)) |
			UsersDatabaseError::Update(UpdateError::LdapError(_)) |
			UsersDatabaseError::Delete(DeleteError::LdapError(_)) => {
				StatusCode::INTERNAL_SERVER_ERROR
			},
		}
	}
}
impl From<UsersDatabaseError> for StatusCode {
	fn from(error: UsersDatabaseError) -> Self {
		StatusCode::from(&error)
	}
}
impl Reply for &UsersDatabaseError {
	fn into_response(self) -> warp::reply::Response {
		StatusCode::from(self).into_response()
	}
}
impl Reply for UsersDatabaseError {
	fn into_response(self) -> warp::reply::Response {
		StatusCode::from(&self).into_response()
	}
}

#[derive(Debug)]
pub enum FetchError {
	ParseError(ParseError),
	LdapError(LdapError),
	MissingPagerData,
	InvalidPage,
	NoItems,
	AmbiguousItems,
	IntegrityError,
}

#[derive(Debug)]
pub enum CreateError {
	LdapError(LdapError),
	AlreadyExists,
}

#[derive(Debug)]
pub enum UpdateError {
	LdapError(LdapError),
	NoItem,
}

#[derive(Debug)]
pub enum DeleteError {
	LdapError(LdapError),
	NoItem,
}

impl From<FetchError> for UsersDatabaseError {
	fn from(value: FetchError) -> Self {
		Self::Fetch(value)
	}
}

impl From<CreateError> for UsersDatabaseError {
	fn from(value: CreateError) -> Self {
		Self::Create(value)
	}
}

impl From<UpdateError> for UsersDatabaseError {
	fn from(value: UpdateError) -> Self {
		Self::Update(value)
	}
}

impl From<DeleteError> for UsersDatabaseError {
	fn from(value: DeleteError) -> Self {
		Self::Delete(value)
	}
}

pub struct UsersDatabase {
	pool: Pool,
}

#[async_trait::async_trait]
impl super::Database for UsersDatabase {
	type Error = PoolError<LdapError>;
	type Connection = UsersConnection;

	async fn connect() -> Result<Self, Self::Error> {
		let url = String::from(CONFIG.ldap_url.as_str());
		let manager = LDAPConnectionManager(url);
		let pool = Pool::builder(manager)
			.build()
			.expect("Failed to start LDAP connection pool");

		Ok(Self { pool })
	}

	async fn connection(&self) -> Result<Self::Connection, Self::Error> {
		self.pool.get().await.map(|connection| UsersConnection { connection })
	}
}

fn user_dn(id: &str) -> String {
	format!(
		"{}={},ou={},{}",
		CONFIG.ldap_users_id_field,
		ldap_escape(id),
		CONFIG.ldap_users_ou,
		CONFIG.ldap_base
	)
}

lazy_static! {
	// TODO: evict periodically
	static ref USER_CACHE: RwLock<HashMap<String, User>> = {
		RwLock::new(HashMap::new())
	};
	static ref PERMISSIONS_CACHE: RwLock<HashMap<Option<String>, EnumSet<Permission>>> = {
		RwLock::new(HashMap::new())
	};
}

pub struct UsersConnection {
	connection: LdapConnection,
}

impl UsersConnection {
	pub async fn list_users(
		&mut self,
		page: LdapPageToken,
		limit: usize,
		filter: crate::routes::users::users::UserFilter,
	) -> Result<Page<Reference<User>>, UsersDatabaseError> {
		let pager = PagedResults {
			size: limit as i32,
			cookie: page.0,
		};

		let mut filters = vec![
			format!("({}=*)", CONFIG.ldap_users_id_field)
		];

		if let Some(ref name) = filter.name {
			filters.push(format!("(displayName={})", name))
		}
		
		if let Some(start) = filter.created_at.start {
			let timestamp = LDAPTimestamp::from(start);
			filters.push(format!("(createTimestamp>={})", String::from(timestamp)));
		}

		if let Some(end) = filter.created_at.end {
			let timestamp = LDAPTimestamp::from(end);
			filters.push(format!("(createTimestamp<={})", String::from(timestamp)));
		}

		let (results, status) = self.connection.with_controls(pager)
			.search(
				&format!("ou={},{}", CONFIG.ldap_users_ou, CONFIG.ldap_base),
				Scope::OneLevel,
				&if filters.len() == 1 {
					filters.pop().unwrap()
				} else {
					format!("(&{})", filters.join(""))
				},
				User::search_fields(),
			).await
			.map_err(FetchError::LdapError)?
			.success()
			// a bit presumptuous, but should be correct enough
			// Update: too presumptuous, this errors if the ou doesn't exist
			// TODO: try to determine error type and use that
			.map_err(|_| FetchError::InvalidPage)?;

		let page_data = status.ctrls.iter()
			.find(|Control(t, _)| matches!(t, Some(ControlType::PagedResults)))
			.map(|Control(_, d)| d.parse::<PagedResults>())
			.ok_or(FetchError::MissingPagerData)?;

		let items = results.into_iter()
			.map(SearchEntry::construct)
			.map(|user| {
				let uid = User::id_from(&user)
					.map_err(ParseError::from)
					.map_err(FetchError::ParseError)?;
				let user = User::try_from(user)
					.map_err(ParseError::from)
					.map_err(FetchError::ParseError)?;
				Ok::<_, FetchError>(Reference::new(User::uri(uid.as_str()), user))
			})
			.collect::<Result<_, _>>()?;
		
			let next = if page_data.cookie.is_empty() {
				None
			} else {
				let page = LdapPageToken(page_data.cookie);
				let mut uri = format!(
					"/users?limit={}&page={}",
					limit, page
				);

				if let Some(name) = filter.name {
					if let Some(name) = byte_serialize(name.as_bytes()).next() {
						uri.push_str(&format!("&name={}", name))
					}
				}
				
				if !filter.created_at.is_open() {
					uri.push_str(&format!("&created_at={}", filter.created_at));
				}

				Some(uri.parse().unwrap())
			};
	
			Ok(Page { items, next, previous: None })	
	}

	pub async fn get_users<'a, S: Into<std::borrow::Cow<'a, str>> + Copy>(
		&mut self,
		ids: &[S],
	) -> Result<HashMap<String, User>, UsersDatabaseError> {
		// TODO: cache

		let mut user_cns = String::new();
		for id in ids {
			user_cns.push_str(&format!("({}={})", CONFIG.ldap_users_id_field, ldap_escape(*id)))
		}

		let (results, _) = self.connection
			.search(
				&format!("ou={},{}", CONFIG.ldap_users_ou, CONFIG.ldap_base),
				Scope::OneLevel,
				&format!("(|{})", user_cns),
				User::search_fields(),
			).await
			.map_err(FetchError::LdapError)?
			.success()
			.map_err(FetchError::LdapError)?;

		results.into_iter()
			.map(SearchEntry::construct)
			.map(|entry| {
				let id = User::id_from(&entry)?;
				let user = User::try_from(entry)?;
				Ok((id, user))
			})
			.collect::<Result<_, _>>()
			.map_err(FetchError::ParseError)
			.map_err(UsersDatabaseError::from)
	}

	pub async fn get_user(
		&mut self,
		id: &str,
	) -> Result<User, UsersDatabaseError> {
		let id_string = id.to_string();
		let cache = USER_CACHE.read().await;
		if let Some(cached) = cache.get(&id_string) {
			return Ok(cached.clone())
		}
		drop(cache);

		let filter = format!("({}={})", CONFIG.ldap_users_id_field, ldap_escape(id));
		let (results, _) = self.connection
			.search(
				&format!("ou={},{}", CONFIG.ldap_users_ou, CONFIG.ldap_base),
				Scope::OneLevel,
				filter.as_str(),
				User::search_fields(),
			).await
			.map_err(FetchError::LdapError)?
			.success()
			.map_err(FetchError::LdapError)?;

		match results.len() {
			0 => Err(FetchError::NoItems.into()),
			1 => {
				let result = results.into_iter()
					.next()
					.map(SearchEntry::construct)
					.unwrap();

				let user = User::try_from(result)
					.map_err(ParseError::from)
					.map_err(FetchError::ParseError)
					.map_err(UsersDatabaseError::from)?;

				let mut cache = USER_CACHE.write().await;
				cache.insert(id_string, user.clone());
				Ok(user)
			},
			_ => Err(FetchError::AmbiguousItems.into()),
		}
	}

	pub async fn update_user(
		&mut self,
		id: &str,
		name: &str,
	) -> Result<(), UsersDatabaseError> {
		let result = self.connection
			.modify(&user_dn(id), vec![
				Mod::Replace("displayName", HashSet::from([name]))
			]).await
			.map_err(UpdateError::LdapError)?;

		let response = match result.rc {
			0 => Ok(()),
			32 => Err(UpdateError::NoItem.into()),
			_ => result.success()
				.map(|_| ())
				.map_err(UpdateError::LdapError)
				.map_err(UsersDatabaseError::from)
		};

		match response {
			Ok(()) => {
				let mut cache = USER_CACHE.write().await;
				cache.remove(&id.to_string());
				Ok(())
			},
			Err(e) => Err(e),
		}
	}

	pub async fn delete_user(
		&mut self,
		id: &str,
	) -> Result<(), UsersDatabaseError> {
		let result = self.connection
			.delete(&user_dn(id)).await
			.map_err(DeleteError::LdapError)?;

		let response = match result.rc {
			0 => Ok(()),
			32 => Err(DeleteError::NoItem.into()),
			_ => result.success()
				.map(|_| ())
				.map_err(DeleteError::LdapError)
				.map_err(UsersDatabaseError::from),
		};

		match response {
			Ok(()) => {
				let mut cache = USER_CACHE.write().await;
				cache.remove(&id.to_string());
				Ok(())
			},
			Err(e) => Err(e),
		}
	}

	pub async fn list_user_roles(
		&mut self,
		id: &str, 
		page: LdapPageToken,
		limit: usize,
		filter: RoleFilter,
	) -> Result<Page<Reference<Role>>, UsersDatabaseError> {
		let pager = PagedResults {
			size: limit as i32,
			cookie: page.0,
		};

		let user_dn = user_dn(id);

		let mut filters = vec![
			if let Some(ref default) = CONFIG.default_role {
				format!("(|(member={})(cn={}))", user_dn, default)
			} else {
				format!("(member={})", user_dn)
			}
		];

		if let Some(ref name) = filter.name {
			filters.push(format!("(cn={})", name))
		}
		
		if let Some(ref icon) = filter.icon {
			filters.push(format!("(pxlsspaceIcon={})", icon));
		}

		let (results, status) = self.connection.with_controls(pager)
			.search(
				&format!("ou={},{}", CONFIG.ldap_roles_ou, CONFIG.ldap_base),
				Scope::OneLevel,
				&if filters.len() == 1 {
					filters.pop().unwrap()
				} else {
					format!("(&{})", filters.join(""))
				},
				Role::search_fields(),
			).await
			.map_err(FetchError::LdapError)?
			.success()
			// a bit presumptuous, but should be correct enough
			.map_err(|_| FetchError::InvalidPage)?;

		let page_data = status.ctrls.iter()
			.find(|Control(t, _)| matches!(t, Some(ControlType::PagedResults)))
			.map(|Control(_, d)| d.parse::<PagedResults>())
			.ok_or(FetchError::MissingPagerData)?;

		let items = results.into_iter()
			.map(SearchEntry::construct)
			.map(Role::try_from)
			.map(|r| r.map(|role| Reference::new(Role::uri(&role.name), role)))
			.map(|r| r.map_err(ParseError::from))
			.map(|r| r.map_err(FetchError::ParseError))
			.collect::<Result<_, _>>()?;

		let next = if page_data.cookie.is_empty() {
			None
		} else {
			let page = LdapPageToken(page_data.cookie);
			let mut uri = format!(
				"/users/{}/roles?limit={}&page={}",
				id, limit, page
			);

			if let Some(name) = filter.name {
				if let Some(name) = byte_serialize(name.as_bytes()).next() {
					uri.push_str(&format!("&name={}", name))
				}
			}
			if let Some(icon) = filter.icon {
				uri.push_str(&format!("&icon={}", icon))
			}

			Some(uri.parse().unwrap())
		};

		Ok(Page { items, next, previous: None })		
	}

	pub async fn list_roles(
		&mut self,
		page: LdapPageToken,
		limit: usize,
		filter: RoleFilter,
	) -> Result<Page<Reference<Role>>, UsersDatabaseError> {
		let pager = PagedResults {
			size: limit as i32,
			cookie: page.0,
		};

		let mut filters = vec![];

		if let Some(ref name) = filter.name {
			filters.push(format!("(cn={})", name))
		}
		
		if let Some(ref icon) = filter.icon {
			filters.push(format!("(pxlsspaceIcon={})", icon));
		}

		let (results, status) = self.connection.with_controls(pager)
			.search(
				&format!("ou={},{}", CONFIG.ldap_roles_ou, CONFIG.ldap_base),
				Scope::OneLevel,
				&if filters.is_empty() {
					"(cn=*)".to_string()
				} else {
					format!("(&{})", filters.join(""))
				},
				Role::search_fields(),
			).await
			.map_err(FetchError::LdapError)?
			.success()
			// a bit presumptuous, but should be correct enough
			.map_err(|_| FetchError::InvalidPage)?;

		let page_data = status.ctrls.iter()
			.find(|Control(t, _)| matches!(t, Some(ControlType::PagedResults)))
			.map(|Control(_, d)| d.parse::<PagedResults>())
			.ok_or(FetchError::MissingPagerData)?;

		let items = results.into_iter()
			.map(SearchEntry::construct)
			.map(Role::try_from)
			.map(|r| r.map(|role| Reference::new(Role::uri(&role.name), role)))
			.map(|r| r.map_err(ParseError::from))
			.map(|r| r.map_err(FetchError::ParseError))
			.collect::<Result<_, _>>()?;
		

		let next = if page_data.cookie.is_empty() {
			None
		} else {
			let page = LdapPageToken(page_data.cookie);
			let mut uri = format!(
				"/roles?limit={}&page={}",
				limit, page
			);

			if let Some(name) = filter.name {
				if let Some(name) = byte_serialize(name.as_bytes()).next() {
					uri.push_str(&format!("&name={}", name))
				}
			}
			if let Some(icon) = filter.icon {
				if let Some(icon) = byte_serialize(icon.as_bytes()).next() {
					uri.push_str(&format!("&icon={}", icon))
				}
			}

			Some(uri.parse().unwrap())
		};

		Ok(Page { items, next, previous: None })
	}

	pub async fn get_role(
		&mut self,
		name: &str,
	) -> Result<Reference<Role>, UsersDatabaseError> {
		let filter = format!("(cn={})", ldap_escape(name));
		let (results, _) = self.connection
			.search(
				&format!("ou={},{}", CONFIG.ldap_roles_ou, CONFIG.ldap_base),
				Scope::OneLevel,
				filter.as_str(),
				Role::search_fields(),
			).await
			.map_err(FetchError::LdapError)?
			.success()
			.map_err(FetchError::LdapError)?;

		match results.len() {
			0 => Err(FetchError::NoItems.into()),
			1 => {
				let result = results.into_iter()
					.next()
					.map(SearchEntry::construct)
					.unwrap();

				Role::try_from(result)
					.map(|role| Reference::new(Role::uri(name), role))
					.map_err(ParseError::from)
					.map_err(FetchError::ParseError)
					.map_err(UsersDatabaseError::from)
			},
			_ => Err(FetchError::AmbiguousItems.into()),
		}
	}

	pub async fn create_role(
		&mut self,
		role: &Role,
	) -> Result<(), UsersDatabaseError> {
		let name = ldap_escape(&role.name).to_string();
		let role_dn = format!(
			"cn={},ou={},{}",
			name,
			CONFIG.ldap_roles_ou,
			CONFIG.ldap_base,
		);

		let mut attributes = vec![];
		
		attributes.push(("objectClass", HashSet::from(["pxlsspaceRole"])));
		attributes.push(("cn", HashSet::from([name.as_str()])));

		let icon = role.icon.as_ref()
			.map(|icon| ldap_escape(icon.as_str()).to_string());
		if let Some(ref icon) = icon {
			let value = HashSet::from([icon.as_str()]);
			attributes.push(("pxlsspaceIcon", value));
		};

		let permissions = role.permissions.iter()
			.map(|p| p.into())
			.collect::<HashSet<&str>>();

		attributes.push(("pxlsspacePermission", permissions));

		let result = self.connection
			.add(&role_dn, attributes).await
			.map_err(CreateError::LdapError)?;

		match result.rc {
			0 => Ok(()),
			68 => Err(CreateError::AlreadyExists.into()),
			_ => result.success()
				.map(|_| ())
				.map_err(CreateError::LdapError)
				.map_err(UsersDatabaseError::from),
		}
	}

	pub async fn update_role(
		&mut self,
		name: &str,
		new_name: Option<&str>,
		icon: Option<Option<Url>>,
		permissions: Option<Vec<Permission>>,
	) -> Result<(), UsersDatabaseError> {
		let mut role_dn = format!(
			"cn={},ou={},{}",
			ldap_escape(name),
			CONFIG.ldap_roles_ou,
			CONFIG.ldap_base,
		);

		if let Some(new_name) = new_name {
			let result = self.connection
				.modifydn(
					role_dn.as_str(),
					format!("cn={}", ldap_escape(new_name)).as_str(),
					true,
					None
				).await
				.map_err(UpdateError::LdapError)?;

			match result.rc {
				0 => (),
				32 => return Err(UpdateError::NoItem.into()),
				_ => result.success()
					.map(|_| ())
					.map_err(UpdateError::LdapError)?,
			}

			role_dn = format!(
				"cn={},ou={},{}",
				ldap_escape(new_name),
				CONFIG.ldap_roles_ou,
				CONFIG.ldap_base,
			);
		}

		let mut modifications = vec![];
		
		let new_icon;
		match icon {
			Some(Some(icon)) => {
				new_icon = ldap_escape(icon.as_str()).to_string();
				let value = HashSet::from([new_icon.as_str()]);
				modifications.push(Mod::Replace("pxlsspaceIcon", value));
			},
			Some(None) => {
				modifications.push(Mod::Delete("pxlsspaceIcon", HashSet::new()));
			}
			None => (),
		}

		let new_permissions = permissions.map(|p| {
			p.iter()
				.map(|p| p.into())
				.collect::<HashSet<&str>>()
		});
		if let Some(new_permissions) = new_permissions {
			modifications.push(Mod::Replace("pxlsspacePermission", new_permissions));
		}

		let result = self.connection
			.modify(&role_dn, modifications).await
			.map_err(UpdateError::LdapError)?;

		let response = match result.rc {
			0 => Ok(()),
			32 => Err(UpdateError::NoItem.into()),
			_ => result.success()
				.map(|_| ())
				.map_err(UpdateError::LdapError)
				.map_err(UsersDatabaseError::from),
		};

		match response {
			Ok(()) => {
				let mut cache = PERMISSIONS_CACHE.write().await;
				cache.clear();
				Ok(())
			},
			Err(e) => Err(e),
		}
	}

	pub async fn delete_role(
		&mut self,
		name: &str,
	) -> Result<(), UsersDatabaseError> {
		let role_dn = format!(
			"cn={},ou={},{}",
			ldap_escape(name),
			CONFIG.ldap_roles_ou,
			CONFIG.ldap_base,
		);

		let result = self.connection
			.delete(&role_dn).await
			.map_err(DeleteError::LdapError)?;

		let response = match result.rc {
			0 => Ok(()),
			32 => Err(DeleteError::NoItem.into()),
			_ => result.success()
				.map(|_| ())
				.map_err(DeleteError::LdapError)
				.map_err(UsersDatabaseError::from),
		};

		match response {
			Ok(()) => {
				let mut cache = PERMISSIONS_CACHE.write().await;
				cache.clear();
				Ok(())
			},
			Err(e) => Err(e),
		}
	}

	pub async fn user_permissions(
		&mut self,
		id: Option<String>,
	) -> Result<EnumSet<Permission>, UsersDatabaseError> {
		let cache = PERMISSIONS_CACHE.read().await;
		if let Some(cached) = cache.get(&id) {
			return Ok(*cached);
		}
		drop(cache);

		let mut filters = vec![];


		if let Some(ref default) = CONFIG.default_role {
			filters.push(format!("(cn={})", default));
		}

		if let Some(user_dn) = id.as_ref().map(|uid| user_dn(uid)) {
			filters.push(format!("(member={})", user_dn))
		}

		let filter = format!("(|{})", filters.join(""));

		let (results, _) = self.connection
			.search(
				&format!("ou={},{}", CONFIG.ldap_roles_ou, CONFIG.ldap_base),
				Scope::OneLevel,
				filter.as_str(),
				["pxlsspacePermission"],
			).await
			.map_err(FetchError::LdapError)?
			.success()
			.map_err(FetchError::LdapError)?;

		let permissions = results.into_iter().flat_map(|result| {
			let permissions = SearchEntry::construct(result);
			permissions.attrs.get("pxlsspacePermission")
				.cloned()
				.unwrap_or_default()
				.into_iter()
				// NOTE: silently drops invalid permissions
				.filter_map(|v| Permission::try_from(v.as_str()).ok())
		}).collect::<EnumSet<Permission>>();

		let mut cache = PERMISSIONS_CACHE.write().await;
		cache.insert(id, permissions);

		Ok(permissions)
	}

	pub async fn add_user_role(
		&mut self,
		uid: &str,
		role: &str,
	) -> Result<(), UsersDatabaseError> {
		let role_dn = format!(
			"cn={},ou={},{}",
			ldap_escape(role),
			CONFIG.ldap_roles_ou,
			CONFIG.ldap_base,
		);

		let result = self.connection
			.modify(role_dn.as_str(), vec![
				Mod::Add("member", HashSet::from([user_dn(uid).as_str()])),
			]).await
			.map_err(UpdateError::LdapError)?;

		let response = match result.rc {
			0 => Ok(()),
			32 => Err(UpdateError::NoItem.into()),
			_ => result.success()
				.map(|_| ())
				.map_err(UpdateError::LdapError)
				.map_err(UsersDatabaseError::from),
		};

		match response {
			Ok(()) => {
				let mut cache = PERMISSIONS_CACHE.write().await;
				cache.remove(&Some(uid.to_string()));
				Ok(())
			},
			Err(e) => Err(e),
		}
	}

	pub async fn remove_user_role(
		&mut self,
		uid: &str,
		role: &str,
	) -> Result<(), UsersDatabaseError> {
		let role_dn = format!(
			"cn={},ou={},{}",
			ldap_escape(role),
			CONFIG.ldap_roles_ou,
			CONFIG.ldap_base,
		);

		let result = self.connection
			.modify(role_dn.as_str(), vec![
				Mod::Delete("member", HashSet::from([user_dn(uid).as_str()])),
			]).await
			.map_err(UpdateError::LdapError)?;

		let response = match result.rc {
			0 => Ok(()),
			32 => Err(UpdateError::NoItem.into()),
			_ => result.success()
				.map(|_| ())
				.map_err(UpdateError::LdapError)
				.map_err(UsersDatabaseError::from),
		};

		match response {
			Ok(()) => {
				let mut cache = PERMISSIONS_CACHE.write().await;
				cache.remove(&Some(uid.to_string()));
				Ok(())
			},
			Err(e) => Err(e),
		}
	}

	pub async fn list_factions(
		&mut self,
		page: LdapPageToken,
		limit: usize,
		filter: FactionFilter,
	) -> Result<Page<Reference<Faction>>, UsersDatabaseError> {
		let pager = PagedResults {
			size: limit as i32,
			cookie: page.0,
		};

		let mut filters = vec![];

		if let Some(ref name) = filter.name {
			let escaped = ldap_escape(name);
			// allow wildcards
			let split = escaped.split("\\2a").collect::<Vec<_>>();
			let last = split.len() - 1;
			let name = split.into_iter()
			.enumerate()
				// but merge consecutive *'s (as they produce an ldap error).
				// We keep the first and last ones always so we have a point to
				// join at.
				.filter(|(i, s)| !s.is_empty() || *i == 0 || *i == last)
				.map(|(_, s)| s)
				.collect::<Vec<_>>()
				.join("*");

			filters.push(format!("(pxlsspaceFactionName={})", name));
		}
		
		if let Some(start) = filter.created_at.start {
			let timestamp = LDAPTimestamp::from(start);
			filters.push(format!("(createTimestamp>={})", String::from(timestamp)));
		}

		if let Some(end) = filter.created_at.end {
			let timestamp = LDAPTimestamp::from(end);
			filters.push(format!("(createTimestamp<={})", String::from(timestamp)));
		}

		// TODO: filter by member count
		// NOTE: this is not trivial because the count is implicit but ldap won't count it for us

		let (results, status) = self.connection.with_controls(pager)
			.search(
				&format!("ou={},{}", CONFIG.ldap_factions_ou, CONFIG.ldap_base),
				Scope::OneLevel,
				&if filters.is_empty() {
					"(cn=*)".to_string()
				} else {
					format!("(&{})", filters.join(""))
				},
				Faction::search_fields(),
			).await
			.map_err(FetchError::LdapError)?
			.success()
			// a bit presumptuous, but should be correct enough
			// Update: too presumptuous, this errors if the ou doesn't exist
			// TODO: try to determine error type and use that
			.map_err(|_| FetchError::InvalidPage)?;

		let page_data = status.ctrls.iter()
			.find(|Control(t, _)| matches!(t, Some(ControlType::PagedResults)))
			.map(|Control(_, d)| d.parse::<PagedResults>())
			.ok_or(FetchError::MissingPagerData)?;

		let items = results.into_iter()
			.map(SearchEntry::construct)
			.map(|f| {
				let fid = Faction::id_from(&f)
					.map_err(ParseError::from)
					.map_err(FetchError::ParseError)?;
				let faction = Faction::try_from(f)
					.map_err(ParseError::from)
					.map_err(FetchError::ParseError)?;
				Ok::<_, FetchError>(Reference::new(Faction::uri(&fid), faction))
			})
			.collect::<Result<_, _>>()?;
		

		let next = if page_data.cookie.is_empty() {
			None
		} else {
			let page = LdapPageToken(page_data.cookie);
			let mut uri = format!(
				"/factions?limit={}&page={}",
				limit, page
			);

			if let Some(name) = filter.name {
				if let Some(name) = byte_serialize(name.as_bytes()).next() {
					uri.push_str(&format!("&name={}", name))
				}
			}
			if !filter.created_at.is_open() {
				uri.push_str(&format!("&created_at={}", filter.created_at))
			}
			if !filter.size.is_open() {
				uri.push_str(&format!("&size={}", filter.size))
			}

			Some(uri.parse().unwrap())
		};

		Ok(Page { items, next, previous: None })
	}

	pub async fn list_user_factions(
		&mut self,
		page: LdapPageToken,
		limit: usize,
		filter: FactionFilter,
		uid: &str,
	) -> Result<Page<Reference<Faction>>, UsersDatabaseError> {
		let pager = PagedResults {
			size: limit as i32,
			cookie: page.0,
		};

		let mut filters = vec![
			format!("(member={})", user_dn(uid)),
		];

		if let Some(ref name) = filter.name {
			filters.push(format!("(cn={})", name))
		}
		
		if let Some(start) = filter.created_at.start {
			let timestamp = LDAPTimestamp::from(start);
			filters.push(format!("(createTimestamp>={})", String::from(timestamp)));
		}

		if let Some(end) = filter.created_at.end {
			let timestamp = LDAPTimestamp::from(end);
			filters.push(format!("(createTimestamp<={})", String::from(timestamp)));
		}

		// TODO: filter by member count
		// NOTE: this is not trivial because the count is implicit but ldap won't count it for us

		let (results, status) = self.connection.with_controls(pager)
			.search(
				&format!("ou={},{}", CONFIG.ldap_factions_ou, CONFIG.ldap_base),
				Scope::OneLevel,
				&if filters.is_empty() {
					"(cn=*)".to_string()
				} else {
					format!("(&{})", filters.join(""))
				},
				Faction::search_fields(),
			).await
			.map_err(FetchError::LdapError)?
			.success()
			// a bit presumptuous, but should be correct enough
			// Update: too presumptuous, this errors if the ou doesn't exist
			// TODO: try to determine error type and use that
			.map_err(|_| FetchError::InvalidPage)?;

		let page_data = status.ctrls.iter()
			.find(|Control(t, _)| matches!(t, Some(ControlType::PagedResults)))
			.map(|Control(_, d)| d.parse::<PagedResults>())
			.ok_or(FetchError::MissingPagerData)?;

		let items = results.into_iter()
			.map(SearchEntry::construct)
			.map(|f| {
				let fid = Faction::id_from(&f)
					.map_err(ParseError::from)
					.map_err(FetchError::ParseError)?;
				let faction = Faction::try_from(f)
					.map_err(ParseError::from)
					.map_err(FetchError::ParseError)?;
				Ok::<_, FetchError>(Reference::new(Faction::uri(&fid), faction))
			})
			.collect::<Result<_, _>>()?;
		

		let next = if page_data.cookie.is_empty() {
			None
		} else {
			let page = LdapPageToken(page_data.cookie);
			let mut uri = format!(
				"/factions?limit={}&page={}",
				limit, page
			);

			if let Some(name) = filter.name {
				if let Some(name) = byte_serialize(name.as_bytes()).next() {
					uri.push_str(&format!("&name={}", name))
				}
			}
			if !filter.created_at.is_open() {
				uri.push_str(&format!("&created_at={}", filter.created_at))
			}
			if !filter.size.is_open() {
				uri.push_str(&format!("&size={}", filter.size))
			}

			Some(uri.parse().unwrap())
		};

		Ok(Page { items, next, previous: None })
	}

	pub async fn get_faction(
		&mut self,
		id: &str,
	) -> Result<Reference<Faction>, UsersDatabaseError> {
		let filter = format!("(cn={})", ldap_escape(id));
		let (results, _) = self.connection
			.search(
				&format!("ou={},{}", CONFIG.ldap_factions_ou, CONFIG.ldap_base),
				Scope::OneLevel,
				filter.as_str(),
				Faction::search_fields(),
			).await
			.map_err(FetchError::LdapError)?
			.success()
			.map_err(FetchError::LdapError)?;

		match results.len() {
			0 => Err(FetchError::NoItems.into()),
			1 => {
				let result = results.into_iter()
					.next()
					.map(SearchEntry::construct)
					.unwrap();

				Faction::try_from(result)
					.map(|faction| Reference::new(Faction::uri(id), faction))
					.map_err(ParseError::from)
					.map_err(FetchError::ParseError)
					.map_err(UsersDatabaseError::from)
			},
			_ => Err(FetchError::AmbiguousItems.into()),
		}
	}

	pub async fn create_faction(
		&mut self,
		name: &str,
	) -> Result<String, UsersDatabaseError> {
		let name = ldap_escape(name).to_string();
		let id = uuid::Uuid::new_v4().to_string();

		let faction_dn = format!(
			"cn={},ou={},{}",
			id,
			CONFIG.ldap_factions_ou,
			CONFIG.ldap_base,
		);

		let mut attributes = vec![];
		
		attributes.push(("objectClass", HashSet::from(["pxlsspaceFaction"])));
		attributes.push(("cn", HashSet::from([id.as_str()])));
		attributes.push(("pxlsspaceFactionName", HashSet::from([name.as_str()])));

		//let icon = icon.as_ref()
		//	.map(|icon| ldap_escape(icon.as_str()).to_string());
		//if let Some(ref icon) = icon {
		//	let value = HashSet::from([icon.as_str()]);
		//	attributes.push(("pxlsspaceIcon", value));
		//};

		let result = self.connection
			.add(&faction_dn, attributes).await
			.map_err(CreateError::LdapError)?;

		match result.rc {
			0 => Ok(id),
			68 => Err(CreateError::AlreadyExists.into()),
			_ => result.success()
				.map(|_| id)
				.map_err(CreateError::LdapError)
				.map_err(UsersDatabaseError::from),
		}
	}

	pub async fn update_faction(
		&mut self,
		name: &str,
		new_name: &str,
	) -> Result<(), UsersDatabaseError> {
		let new_name = ldap_escape(new_name).to_string();

		let faction_dn = format!(
			"cn={},ou={},{}",
			ldap_escape(name),
			CONFIG.ldap_factions_ou,
			CONFIG.ldap_base,
		);

		let mut modifications = vec![];
		
		modifications.push(Mod::Replace("pxlsspaceFactionName", HashSet::from([new_name.as_str()])));

		let result = self.connection
			.modify(&faction_dn, modifications).await
			.map_err(UpdateError::LdapError)?;

		match result.rc {
			0 => Ok(()),
			32 => Err(UpdateError::NoItem.into()),
			_ => result.success()
				.map(|_| ())
				.map_err(UpdateError::LdapError)
				.map_err(UsersDatabaseError::from),
		}
	}

	pub async fn delete_faction(
		&mut self,
		id: &str,
	) -> Result<(), UsersDatabaseError> {
		let faction_dn = format!(
			"cn={},ou={},{}",
			ldap_escape(id),
			CONFIG.ldap_factions_ou,
			CONFIG.ldap_base,
		);

		let result = self.connection
			.delete(&faction_dn).await
			.map_err(DeleteError::LdapError)?;

		match result.rc {
			0 => Ok(()),
			32 => Err(DeleteError::NoItem.into()),
			_ => result.success()
				.map(|_| ())
				.map_err(DeleteError::LdapError)
				.map_err(UsersDatabaseError::from),
		}
	}

	pub async fn list_faction_members(
		&mut self,
		fid: &str,
		page: LdapPageToken,
		limit: usize,
		filter: MemberFilter, // TODO: filter all of this
	) -> Result<Page<Reference<FactionMember>>, UsersDatabaseError> {
		let pager = PagedResults {
			size: limit as i32,
			cookie: page.0,
		};

		let (results, status) = self.connection.with_controls(pager)
			.search(
				&format!("ou={},{}", CONFIG.ldap_factions_ou, CONFIG.ldap_base),
				Scope::OneLevel,
				&format!("(cn={})", ldap_escape(fid)),
				["member", "pxlsspaceFactionOwner"],
			).await
			.map_err(FetchError::LdapError)?
			.success()
			// a bit presumptuous, but should be correct enough
			// Update: too presumptuous, this errors if the ou doesn't exist
			// TODO: try to determine error type and use that
			.map_err(|_| FetchError::InvalidPage)?;

		let page_data = status.ctrls.iter()
			.find(|Control(t, _)| matches!(t, Some(ControlType::PagedResults)))
			.map(|Control(_, d)| d.parse::<PagedResults>())
			.ok_or(FetchError::MissingPagerData)?;

		let entry = results.into_iter()
			.map(SearchEntry::construct)
			.next()
			.ok_or_else(|| FetchError::NoItems)?;

		let owners = entry.attrs.get("pxlsspaceFactionOwner")
			.cloned()
			.unwrap_or_default();
		
		let empty = vec![];
		let user_cns = entry.attrs.get("member").unwrap_or(&empty);
		let user_ids = user_cns.iter()
			.map(|cn| &cn.split_once(',').unwrap().0[4..])
			.collect::<Vec<_>>();

		let users = self.get_users(&user_ids).await?;
		let items = entry.attrs.get("member")
			.unwrap_or(&empty)
			.iter()
			.map(|cn| {
				let owner = owners.contains(cn);
				let uid = &cn.split_once(',').unwrap().0[4..];
				let join_intent = JoinIntent::default();
				let user = users.get(uid)
					.ok_or(FetchError::IntegrityError)
					.map(|u| Reference::new(User::uri(uid), u.clone()))?;
				let member = FactionMember { owner, join_intent, user };
				let uri = FactionMember::uri(fid, uid);
				Ok(Reference::new(uri, member))
			})
			.collect::<Result<Vec<_>, FetchError>>()?;

		let next = if page_data.cookie.is_empty() {
			None
		} else {
			let page = LdapPageToken(page_data.cookie);
			let mut uri = format!(
				"/factions/{}/members?limit={}&page={}",
				fid, limit, page
			);

			if let Some(owner) = filter.owner {
				uri.push_str(&format!("&owner={}", owner))
			}

			Some(uri.parse().unwrap())
		};

		Ok(Page { items, next, previous: None })
	}
	
	pub async fn get_faction_owners(
		&mut self,
		fid: &str,
	) -> Result<Vec<String>, UsersDatabaseError> {
		let (results, _) = self.connection
			.search(
				&format!("ou={},{}", CONFIG.ldap_factions_ou, CONFIG.ldap_base),
				Scope::OneLevel,
				&format!("(cn={})", ldap_escape(fid)),
				["pxlsspaceFactionOwner"],
			).await
			.map_err(FetchError::LdapError)?
			.success()
			// a bit presumptuous, but should be correct enough
			// Update: too presumptuous, this errors if the ou doesn't exist
			// TODO: try to determine error type and use that
			.map_err(|_| FetchError::InvalidPage)?;

		let entry = results.into_iter()
			.map(SearchEntry::construct)
			.next()
			.ok_or_else(|| FetchError::NoItems)?;

		
		let empty = vec![];
		let user_cns = entry.attrs.get("pxlsspaceFactionOwner").unwrap_or(&empty);
		let user_ids = user_cns.iter()
			.map(|cn| &cn.split_once(',').unwrap().0[4..])
			.map(|s| s.to_owned())
			.collect::<Vec<_>>();

		Ok(user_ids)
	}
	
	pub async fn get_all_faction_members(
		&mut self,
		fid: &str,
	) -> Result<Vec<String>, UsersDatabaseError> {
		let (results, _) = self.connection
			.search(
				&format!("ou={},{}", CONFIG.ldap_factions_ou, CONFIG.ldap_base),
				Scope::OneLevel,
				&format!("(cn={})", ldap_escape(fid)),
				["member"],
			).await
			.map_err(FetchError::LdapError)?
			.success()
			// a bit presumptuous, but should be correct enough
			// Update: too presumptuous, this errors if the ou doesn't exist
			// TODO: try to determine error type and use that
			.map_err(|_| FetchError::InvalidPage)?;

		let entry = results.into_iter()
			.map(SearchEntry::construct)
			.next()
			.ok_or_else(|| FetchError::NoItems)?;

		
		let empty = vec![];
		let user_cns = entry.attrs.get("member").unwrap_or(&empty);
		let user_ids = user_cns.iter()
			.map(|cn| &cn.split_once(',').unwrap().0[4..])
			.map(|s| s.to_owned())
			.collect::<Vec<_>>();

		Ok(user_ids)
	}

	pub async fn get_faction_member(
		&mut self,
		fid: &str,
		uid: &str,
	) -> Result<Reference<FactionMember>, UsersDatabaseError> {
		let user_dn = user_dn(uid);
		let user = self.get_user(uid).await?;
		let user = Reference::new(User::uri(uid), user);

		let (entries, _result) = self.connection
			.search(
				&format!("ou={},{}", CONFIG.ldap_factions_ou, CONFIG.ldap_base),
				Scope::OneLevel,
				&format!("(cn={})", ldap_escape(fid)),
				["member", "pxlsspaceFactionOwner"],
			).await
			.map_err(FetchError::LdapError)?
			.success()
			.map_err(FetchError::LdapError)?;

		entries.into_iter()
			.next()
			.ok_or(FetchError::NoItems)
			.map(SearchEntry::construct)
			.and_then(|e| {
				let member = e.attrs.get("member")
					.cloned()
					.unwrap_or_default()
					.contains(&user_dn);

				if !member {
					return Err(FetchError::NoItems)
				}

				let owner = e.attrs.get("pxlsspaceFactionOwner")
					.cloned()
					.unwrap_or_default()
					.contains(&user_dn);

				let join_intent = JoinIntent::default();
				let member = FactionMember { owner, join_intent, user };
				let uri = FactionMember::uri(fid, uid);
				Ok(Reference::new(uri, member))
			})
			.map_err(UsersDatabaseError::from)
	}

	pub async fn add_faction_member(
		&mut self,
		fid: &str,
		uid: &str,
		owner: bool,
	) -> Result<Reference<FactionMember>, UsersDatabaseError> {
		// verify that user exists (because ldap doesn't do that I guess)
		let user = self.get_user(uid).await?;
		let user = Reference::new(User::uri(uid), user);

		let faction_dn = format!(
			"cn={},ou={},{}",
			ldap_escape(fid),
			CONFIG.ldap_factions_ou,
			CONFIG.ldap_base,
		);

		let user_dn = user_dn(uid);
		
		let mut attributes = vec![];
		attributes.push(Mod::Add("member", HashSet::from([user_dn.as_str()])));
		if owner {
			attributes.push(Mod::Add("pxlsspaceFactionOwner", HashSet::from([user_dn.as_str()])));
		}

		let result = self.connection
			.modify(faction_dn.as_str(), attributes).await
			.map_err(UpdateError::LdapError)?;

		let uri = FactionMember::uri(fid, uid);
		let join_intent = JoinIntent::default();
		let member = FactionMember { owner, join_intent, user };
		let reference = Reference::new(uri, member);

		match result.rc {
			0 => Ok(reference),
			20 => Err(CreateError::AlreadyExists.into()),
			32 => Err(UpdateError::NoItem.into()),
			_ => result.success()
				.map(|_| reference)
				.map_err(UpdateError::LdapError)
				.map_err(UsersDatabaseError::from),
		}
	}

	pub async fn edit_faction_member(
		&mut self,
		fid: &str,
		uid: &str,
		owner: bool,
	) -> Result<Reference<FactionMember>, UsersDatabaseError> {
		// verify that user exists (because ldap doesn't do that I guess)
		let user = self.get_user(uid).await?;
		let user = Reference::new(User::uri(uid), user);

		let faction_dn = format!(
			"cn={},ou={},{}",
			ldap_escape(fid),
			CONFIG.ldap_factions_ou,
			CONFIG.ldap_base,
		);

		let user_dn = user_dn(uid);
		
		let mut attributes = vec![];
		if owner {
			attributes.push(Mod::Add("pxlsspaceFactionOwner", HashSet::from([user_dn.as_str()])));
		} else {
			attributes.push(Mod::Delete("pxlsspaceFactionOwner", HashSet::from([user_dn.as_str()])));
		}

		let result = self.connection
			.modify(faction_dn.as_str(), attributes).await
			.map_err(UpdateError::LdapError)?;

		let uri = FactionMember::uri(fid, uid);
		let join_intent = JoinIntent::default();
		let member = FactionMember { owner, join_intent, user };
		let reference = Reference::new(uri, member);

		match result.rc {
			0 => Ok(reference),
			20 => Ok(reference), // TODO: this might not be correct (20 = already exists)
			32 => Err(UpdateError::NoItem.into()),
			_ => result.success()
				.map(|_| reference)
				.map_err(UpdateError::LdapError)
				.map_err(UsersDatabaseError::from),
		}
	}

	pub async fn remove_faction_member(
		&mut self,
		fid: &str,
		uid: &str,
	) -> Result<(), UsersDatabaseError> {
		let faction_dn = format!(
			"cn={},ou={},{}",
			ldap_escape(fid),
			CONFIG.ldap_factions_ou,
			CONFIG.ldap_base,
		);

		let user_dn = user_dn(uid);

		let owner_attribute = vec![
			Mod::Delete("pxlsspaceFactionOwner", HashSet::from([user_dn.as_str()]))
		];
		let result = self.connection
			.modify(faction_dn.as_str(), owner_attribute).await
			.map_err(UpdateError::LdapError)?;

		// try deleting owner first so that we don't error if there's no owner
		match result.rc {
			0 => Ok(()),
			16 => Ok(()), // no such attribute (no owner)
			32 => Err(UpdateError::NoItem.into()),
			_ => result.success()
				.map(|_| ())
				.map_err(UpdateError::LdapError)
				.map_err(UsersDatabaseError::from)
		}?;
		
		let member_attribute = vec![
			Mod::Delete("member", HashSet::from([user_dn.as_str()])),
		];

		let result = self.connection
			.modify(faction_dn.as_str(), member_attribute).await
			.map_err(UpdateError::LdapError)?;

		match result.rc {
			0 => Ok(()),
			16 | 32 => Err(UpdateError::NoItem.into()),
			_ => result.success()
				.map(|_| ())
				.map_err(UpdateError::LdapError)
				.map_err(UsersDatabaseError::from),
		}
	}
}