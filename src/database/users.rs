use std::{collections::HashSet, fmt};

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

pub use entities::{User, Role, ParseError};
use connection::Connection as LdapConnection;
use connection::LDAPConnectionManager;
use connection::Pool;
use reqwest::StatusCode;
use serde::de::{Deserialize, Visitor};
use url::Url;
use warp::{reject::Reject, reply::Reply};

use crate::config::CONFIG;
use crate::permissions::Permission;
use crate::filter::response::paginated_list::{Page, PageToken};

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
		error.into()
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
		self.pool.get().await
			.map(|connection| UsersConnection { connection })
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

pub struct UsersConnection {
	connection: LdapConnection,
}

impl UsersConnection {
	pub async fn list_users(
		&mut self,
		page: LdapPageToken,
		limit: usize,
	) -> Result<Page<User>, UsersDatabaseError> {
		let pager = PagedResults {
			size: limit as i32,
			cookie: page.0,
		};

		let filter = format!("({}=*)", CONFIG.ldap_users_id_field);
		let (results, status) = self.connection.with_controls(pager)
			.search(
				&format!("ou={},{}", CONFIG.ldap_users_ou, CONFIG.ldap_base),
				Scope::OneLevel,
				filter.as_str(),
				User::search_fields(),
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
			.map(User::try_from)
			.map(|r| r.map_err(ParseError::from))
			.map(|r| r.map_err(FetchError::ParseError))
			.collect::<Result<_, _>>()?;
		
			let next = if page_data.cookie.is_empty() {
				None
			} else {
				let page = LdapPageToken(page_data.cookie);
				Some(format!(
					"/users?limit={}&page={}",
					limit, page
				).parse().unwrap())
			};
	
			Ok(Page { items, next, previous: None })	
	}

	pub async fn get_user(
		&mut self,
		id: &str,
	) -> Result<User, UsersDatabaseError> {
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

				User::try_from(result)
					.map_err(ParseError::from)
					.map_err(FetchError::ParseError)
					.map_err(UsersDatabaseError::from)
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

		match result.rc {
			0 => Ok(()),
			32 => Err(UpdateError::NoItem.into()),
			_ => result.success()
				.map(|_| ())
				.map_err(UpdateError::LdapError)
				.map_err(UsersDatabaseError::from)
		}
	}

	pub async fn delete_user(
		&mut self,
		id: &str,
	) -> Result<(), UsersDatabaseError> {
		let result = self.connection
			.delete(&user_dn(id)).await
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

	pub async fn list_user_roles(
		&mut self,
		id: &str, 
		page: LdapPageToken,
		limit: usize,
	) -> Result<Page<Role>, UsersDatabaseError> {
		let pager = PagedResults {
			size: limit as i32,
			cookie: page.0,
		};

		let user_dn = format!(
			"{}={},ou={},{}",
			CONFIG.ldap_users_id_field,
			ldap_escape(id),
			CONFIG.ldap_users_ou,
			CONFIG.ldap_base,
		);
		let filter = format!("(member={})", user_dn);
		let (results, status) = self.connection.with_controls(pager)
			.search(
				&format!("ou={},{}", CONFIG.ldap_roles_ou, CONFIG.ldap_base),
				Scope::OneLevel,
				filter.as_str(),
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
			.map(|r| r.map_err(ParseError::from))
			.map(|r| r.map_err(FetchError::ParseError))
			.collect::<Result<_, _>>()?;

		let next = if page_data.cookie.is_empty() {
			None
		} else {
			let page = LdapPageToken(page_data.cookie);
			Some(format!(
				"/users/{}/roles?limit={}&page={}",
				id, limit, page
			).parse().unwrap())
		};

		Ok(Page { items, next, previous: None })		
	}

	pub async fn list_roles(
		&mut self,
		page: LdapPageToken,
		limit: usize,
	) -> Result<Page<Role>, UsersDatabaseError> {
		let pager = PagedResults {
			size: limit as i32,
			cookie: page.0,
		};

		let (results, status) = self.connection.with_controls(pager)
			.search(
				&format!("ou={},{}", CONFIG.ldap_roles_ou, CONFIG.ldap_base),
				Scope::OneLevel,
				"(cn=*)",
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
			.map(|r| r.map_err(ParseError::from))
			.map(|r| r.map_err(FetchError::ParseError))
			.collect::<Result<_, _>>()?;
		

		let next = if page_data.cookie.is_empty() {
			None
		} else {
			let page = LdapPageToken(page_data.cookie);
			Some(format!(
				"/roles?limit={}&page={}",
				limit, page
			).parse().unwrap())
		};

		Ok(Page { items, next, previous: None })
	}

	pub async fn get_role(
		&mut self,
		name: &str,
	) -> Result<Role, UsersDatabaseError> {
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

		match result.rc {
			0 => Ok(()),
			32 => Err(UpdateError::NoItem.into()),
			_ => result.success()
				.map(|_| ())
				.map_err(UpdateError::LdapError)
				.map_err(UsersDatabaseError::from),
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

		match result.rc {
			0 => Ok(()),
			32 => Err(DeleteError::NoItem.into()),
			_ => result.success()
				.map(|_| ())
				.map_err(DeleteError::LdapError)
				.map_err(UsersDatabaseError::from),
		}
	}

	pub async fn user_permissions(
		&mut self,
		id: &str,
	) -> Result<EnumSet<Permission>, UsersDatabaseError> {
		let filter = format!("(member={})", user_dn(id));
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

		Ok(results.into_iter().flat_map(|result| {
			let permissions = SearchEntry::construct(result);
			permissions.attrs.get("pxlsspacePermission")
				.cloned()
				.unwrap_or_default()
				.into_iter()
				// NOTE: silently drops invalid permissions
				.filter_map(|v| Permission::try_from(v.as_str()).ok())
		}).collect())
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

		match result.rc {
			0 => Ok(()),
			32 => Err(UpdateError::NoItem.into()),
			_ => result.success()
				.map(|_| ())
				.map_err(UpdateError::LdapError)
				.map_err(UsersDatabaseError::from),
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

		match result.rc {
			0 => Ok(()),
			32 => Err(UpdateError::NoItem.into()),
			_ => result.success()
				.map(|_| ())
				.map_err(UpdateError::LdapError)
				.map_err(UsersDatabaseError::from),
		}
	}
}