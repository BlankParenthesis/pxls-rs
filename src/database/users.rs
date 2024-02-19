use base64::prelude::*;
use deadpool::managed::PoolError;
use ldap3::{
	LdapError,
	controls::{PagedResults, Control, ControlType},
	Scope,
	SearchEntry,
	ldap_escape,
};

mod connection;
mod entities;

use entities::{User, UserParseError, Role, RoleParseError};
use connection::Connection as LdapConnection;
use connection::LDAPConnectionManager;
use connection::Pool;

use crate::config::CONFIG;


#[derive(Debug)]
pub enum FetchError<E> {
	ParseError(E),
	LdapError(LdapError),
	MissingPagerData,
	InvalidPage,
	NoItems,
	AmbiguousItems,
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

pub struct UsersConnection {
	connection: LdapConnection,
}

impl UsersConnection {
	pub async fn list_users(
		&mut self,
		page: PageToken,
		limit: usize,
	) -> Result<(PageToken, Vec<User>), FetchError<UserParseError>> {
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
	) -> Result<User, FetchError<UserParseError>> {
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
			0 => Err(FetchError::NoItems),
			1 => {
				let result = results.into_iter()
					.next()
					.map(SearchEntry::construct)
					.unwrap();

				User::try_from(result)
					.map_err(FetchError::ParseError)
			},
			_ => Err(FetchError::AmbiguousItems),
		}
	}

	pub async fn list_user_roles(
		&mut self,
		id: &str, 
		page: PageToken,
		limit: usize,
	) -> Result<(PageToken, Vec<Role>), FetchError<RoleParseError>> {
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
			.map(|r| r.map_err(FetchError::ParseError))
			.collect::<Result<_, _>>()?;
		
		if page_data.cookie.is_empty() {
			Ok((None, items))
		} else {
			let page = BASE64_URL_SAFE.encode(page_data.cookie);
			Ok((Some(page), items))
		}
	}

	pub async fn list_roles(
		&mut self,
		page: PageToken,
		limit: usize,
	) -> Result<(PageToken, Vec<Role>), FetchError<RoleParseError>> {
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
	) -> Result<Role, FetchError<RoleParseError>> {
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
			0 => Err(FetchError::NoItems),
			1 => {
				let result = results.into_iter()
					.next()
					.map(SearchEntry::construct)
					.unwrap();

				Role::try_from(result)
					.map_err(FetchError::ParseError)
			},
			_ => Err(FetchError::AmbiguousItems),
		}
	}
}