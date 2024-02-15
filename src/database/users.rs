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

use entities::{User, UserParseError};
use connection::Connection as LdapConnection;
use connection::LDAPConnectionManager;
use connection::Pool;

use crate::config::CONFIG;


#[derive(Debug)]
pub enum UserFetchError {
	ParseError(UserParseError),
	LdapError(LdapError),
	MissingPagerData,
	InvalidPage,
	MissingUser,
	AmbiguousUser,
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
		let url = String::from(CONFIG.users_ldap_url.as_str());
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
	) -> Result<(PageToken, Vec<User>), UserFetchError> {
		let pager = PagedResults {
			size: limit as i32,
			cookie: page.map(|p| BASE64_URL_SAFE.decode(p))
				.unwrap_or(Ok(vec![]))
				.map_err(|_| UserFetchError::InvalidPage)?,
		};

		let filter = format!("({}=*)", CONFIG.users_ldap_id_field);
		let (results, status) = self.connection.with_controls(pager)
			.search(
				&CONFIG.users_ldap_base,
				Scope::OneLevel,
				filter.as_str(),
				[
					&CONFIG.users_ldap_id_field,
					&CONFIG.users_ldap_username_field,
					"createTimestamp"
				],
			).await
			.map_err(UserFetchError::LdapError)?
			.success()
			// a bit presumptuous, but should be correct enough
			.map_err(|_| UserFetchError::InvalidPage)?;

		let page_data = status.ctrls.iter()
			.find(|Control(t, _)| matches!(t, Some(ControlType::PagedResults)))
			.map(|Control(_, d)| d.parse::<PagedResults>())
			.ok_or(UserFetchError::MissingPagerData)?;

		let items = results.into_iter()
			.map(SearchEntry::construct)
			.map(User::try_from)
			.map(|r| r.map_err(UserFetchError::ParseError))
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
		uid: String,
	) -> Result<User, UserFetchError> {
		let filter = format!("({}={})", CONFIG.users_ldap_id_field, ldap_escape(uid));
		let (results, _) = self.connection
			.search(
				&CONFIG.users_ldap_base,
				Scope::OneLevel,
				filter.as_str(),
				[
					&CONFIG.users_ldap_id_field,
					&CONFIG.users_ldap_username_field,
					"createTimestamp"
				],
			).await
			.map_err(UserFetchError::LdapError)?
			.success()
			.map_err(UserFetchError::LdapError)?;

		match results.len() {
			0 => Err(UserFetchError::MissingUser),
			1 => {
				let result = results.into_iter()
					.next()
					.map(SearchEntry::construct)
					.unwrap();

				User::try_from(result)
					.map_err(UserFetchError::ParseError)
			},
			_ => Err(UserFetchError::AmbiguousUser),
		}
	}
}