use std::collections::HashSet;

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
use url::Url;
use warp::{reject::Reject, reply::Reply};

use crate::{config::CONFIG, permissions::Permission};

// TODO: rename
#[derive(Debug)]
pub enum DatabaseError {
	Fetch(FetchError),
	Create(CreateError),
	Update(UpdateError),
	Delete(DeleteError),
}

impl Reject for DatabaseError {}
impl Reply for &DatabaseError {
	fn into_response(self) -> warp::reply::Response {
		match self {
			DatabaseError::Update(UpdateError::NoItem) |
			DatabaseError::Delete(DeleteError::NoItem) |
			DatabaseError::Fetch(FetchError::NoItems) => {
				StatusCode::NOT_FOUND.into_response()
			},
			DatabaseError::Fetch(FetchError::InvalidPage) => {
				StatusCode::BAD_REQUEST.into_response()
			},
			DatabaseError::Create(CreateError::AlreadyExists) => {
				StatusCode::CONFLICT.into_response()
			},
			DatabaseError::Fetch(FetchError::MissingPagerData) |
			DatabaseError::Fetch(FetchError::AmbiguousItems) |
			DatabaseError::Fetch(FetchError::ParseError(_)) |
			DatabaseError::Fetch(FetchError::LdapError(_)) |
			DatabaseError::Create(CreateError::LdapError(_)) |
			DatabaseError::Update(UpdateError::LdapError(_)) |
			DatabaseError::Delete(DeleteError::LdapError(_)) => {
				StatusCode::INTERNAL_SERVER_ERROR.into_response()
			},
		}
	}
}
impl Reply for DatabaseError {
	fn into_response(self) -> warp::reply::Response {
		(&self).into_response()
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

impl From<FetchError> for DatabaseError {
	fn from(value: FetchError) -> Self {
		Self::Fetch(value)
	}
}

impl From<CreateError> for DatabaseError {
	fn from(value: CreateError) -> Self {
		Self::Create(value)
	}
}

impl From<UpdateError> for DatabaseError {
	fn from(value: UpdateError) -> Self {
		Self::Update(value)
	}
}

impl From<DeleteError> for DatabaseError {
	fn from(value: DeleteError) -> Self {
		Self::Delete(value)
	}
}

pub struct UsersDatabase {
	pool: Pool,
}

pub type PageToken = Option<String>;

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
		page: PageToken,
		limit: usize,
	) -> Result<(PageToken, Vec<User>), DatabaseError> {
		let pager = PagedResults {
			size: limit as i32,
			cookie: page.map(|p| BASE64_URL_SAFE.decode(p))
				.unwrap_or(Ok(vec![]))
				.map_err(|_| FetchError::InvalidPage)?,
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
		
		if page_data.cookie.is_empty() {
			Ok((None, items))
		} else {
			let page = BASE64_URL_SAFE.encode(page_data.cookie);
			Ok((Some(page), items))
		}
	}

	pub async fn get_user(
		&mut self,
		id: &str,
	) -> Result<User, DatabaseError> {
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
					.map_err(DatabaseError::from)
			},
			_ => Err(FetchError::AmbiguousItems.into()),
		}
	}

	pub async fn update_user(
		&mut self,
		id: &str,
		name: &str,
	) -> Result<(), DatabaseError> {
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
				.map_err(DatabaseError::from)
		}
	}

	pub async fn delete_user(
		&mut self,
		id: &str,
	) -> Result<(), DatabaseError> {
		let result = self.connection
			.delete(&user_dn(id)).await
			.map_err(DeleteError::LdapError)?;

		match result.rc {
			0 => Ok(()),
			32 => Err(DeleteError::NoItem.into()),
			_ => result.success()
				.map(|_| ())
				.map_err(DeleteError::LdapError)
				.map_err(DatabaseError::from),
		}
	}

	pub async fn list_user_roles(
		&mut self,
		id: &str, 
		page: PageToken,
		limit: usize,
	) -> Result<(PageToken, Vec<Role>), DatabaseError> {
		let pager = PagedResults {
			size: limit as i32,
			cookie: page.map(|p| BASE64_URL_SAFE.decode(p))
				.unwrap_or(Ok(vec![]))
				.map_err(|_| FetchError::InvalidPage)?,
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
		
		if page_data.cookie.is_empty() {
			Ok((None, items))
		} else {
			let page = BASE64_URL_SAFE.encode(page_data.cookie);
			Ok((Some(page), items))
		}
	}

	// TODO: consider returning Page
	pub async fn list_roles(
		&mut self,
		page: PageToken,
		limit: usize,
	) -> Result<(PageToken, Vec<Role>), DatabaseError> {
		let pager = PagedResults {
			size: limit as i32,
			cookie: page.map(|p| BASE64_URL_SAFE.decode(p))
				.unwrap_or(Ok(vec![]))
				.map_err(|_| FetchError::InvalidPage)?,
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
		
		if page_data.cookie.is_empty() {
			Ok((None, items))
		} else {
			let page = BASE64_URL_SAFE.encode(page_data.cookie);
			Ok((Some(page), items))
		}
	}

	pub async fn get_role(
		&mut self,
		name: &str,
	) -> Result<Role, DatabaseError> {
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
					.map_err(DatabaseError::from)
			},
			_ => Err(FetchError::AmbiguousItems.into()),
		}
	}

	pub async fn create_role(
		&mut self,
		role: &Role,
	) -> Result<(), DatabaseError> {
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
				.map_err(DatabaseError::from),
		}
	}

	pub async fn update_role(
		&mut self,
		name: &str,
		new_name: Option<&str>,
		icon: Option<Option<Url>>,
		permissions: Option<Vec<Permission>>,
	) -> Result<(), DatabaseError> {
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
				.map_err(DatabaseError::from),
		}
	}

	pub async fn delete_role(
		&mut self,
		name: &str,
	) -> Result<(), DatabaseError> {
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
				.map_err(DatabaseError::from),
		}
	}

	pub async fn user_permissions(
		&mut self,
		id: &str,
	) -> Result<EnumSet<Permission>, DatabaseError> {
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
	) -> Result<(), DatabaseError> {
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
				.map_err(DatabaseError::from),
		}
	}

	pub async fn remove_user_role(
		&mut self,
		uid: &str,
		role: &str,
	) -> Result<(), DatabaseError> {
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
				.map_err(DatabaseError::from),
		}
	}
}