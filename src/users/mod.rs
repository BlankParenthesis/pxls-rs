use async_trait::async_trait;
use ldap3::{LdapConnAsync, LdapError, drive, Ldap, SearchEntry, Scope, controls::{PagedResults, Control, ControlType}, ldap_escape};

use deadpool::managed::{Manager, Metrics, RecycleResult};
use serde::Serialize;

use base64::prelude::*;

use crate::config::CONFIG;

pub type Pool = deadpool::managed::Pool<LDAPConnectionManager>;
pub type Connection = deadpool::managed::Object<LDAPConnectionManager>;
pub struct LDAPConnectionManager(pub String);

#[async_trait]
impl Manager for LDAPConnectionManager {
	type Type = Ldap;
	type Error = LdapError;

	async fn create(&self) -> Result<Self::Type, Self::Error> {
		let (connection, ldap) = LdapConnAsync::new(&self.0).await?;
		drive!(connection);
		Ok(ldap)
	}

	async fn recycle(
		&self,
		_connection: &mut Self::Type,
		_metrics: &Metrics,
	) -> RecycleResult<Self::Error> {
		// TODO: maybe the connection should be checked for errors?
		// the r2d2 crate runs a whoami query to check this, but doing that
		// for every connection pulled out of the pool seems very wasteful.
		Ok(())
	}
}

struct LDAPTimestamp {
	year: i32,
	month: u32,
	day: u32,
	hour: u32,
	minute: u32,
	second: u32,
}

impl TryFrom<&str> for LDAPTimestamp {
	type Error = UserParseError;

	fn try_from(value: &str) -> Result<Self, Self::Error> {
		if value.len() != 15 {
			return Err(UserParseError::IncorrectTimestampLength);
		}
		if &value[14..15] != "Z" {
			return Err(UserParseError::IncorrectTimestampSuffix);
		}

		let year = value[0..4].parse()
			.map_err(|_| UserParseError::IncorrectTimestampYear)?;
		let month = value[4..6].parse()
			.map_err(|_| UserParseError::IncorrectTimestampMonth)?;
		let day = value[6..8].parse()
			.map_err(|_| UserParseError::IncorrectTimestampDay)?;
		let hour = value[8..10].parse()
			.map_err(|_| UserParseError::IncorrectTimestampHour)?;
		let minute = value[10..12].parse()
			.map_err(|_| UserParseError::IncorrectTimestampMinute)?;
		let second = value[12..14].parse()
			.map_err(|_| UserParseError::IncorrectTimestampSecond)?;

		Ok(Self { year, month, day, hour, minute, second })
	}
}

impl TryFrom<LDAPTimestamp> for u64 {
	type Error = UserParseError;

	fn try_from(value: LDAPTimestamp) -> Result<Self, Self::Error> {
		Ok(chrono::NaiveDate::from_ymd_opt(value.year, value.month, value.day)
			.ok_or(UserParseError::InvalidDate)?
			.and_hms_opt(value.hour, value.minute, value.second)
			.ok_or(UserParseError::InvalidTime)?
			.signed_duration_since(chrono::NaiveDateTime::UNIX_EPOCH)
			.num_seconds() as u64)
	}
}

#[derive(Debug)]
pub enum UserParseError {
	MissingUid,
	MissingTimestamp,
	IncorrectTimestampLength,
	IncorrectTimestampSuffix,
	IncorrectTimestampYear,
	IncorrectTimestampMonth,
	IncorrectTimestampDay,
	IncorrectTimestampHour,
	IncorrectTimestampMinute,
	IncorrectTimestampSecond,
	InvalidDate,
	InvalidTime,
}

#[derive(Debug)]
pub enum UserFetchError {
	ParseError(UserParseError),
	LdapError(LdapError),
	MissingPagerData,
	InvalidPage,
	MissingUser,
	AmbiguousUser,
}

// TODO: this should live in crate::objects probably:
// consolidate naming with user objects
#[derive(Debug, Serialize)]
pub struct User {
	#[serde(skip_serializing)]
	pub id: String,
	pub name: String,
	pub created_at: u64,
}

impl TryFrom<SearchEntry> for User {
	type Error = UserParseError;

	fn try_from(value: SearchEntry) -> Result<Self, Self::Error> {
		let id = value.attrs.get(&CONFIG.users_ldap_id_field)
			.and_then(|v| v.first())
			.ok_or(UserParseError::MissingUid)?
			.to_owned();
		let name = value.attrs.get(&CONFIG.users_ldap_username_field)
			.and_then(|v| v.first())
			.ok_or(UserParseError::MissingUid)?
			.to_owned();
		let created_at = value.attrs.get("createTimestamp")
			.and_then(|v| v.first())
			.ok_or(UserParseError::MissingTimestamp)
			.and_then(|s| LDAPTimestamp::try_from(s.as_str()))
			.and_then(u64::try_from)?;

		Ok(User{ id, name, created_at })
	}
}

pub async fn list(
	connection: &mut Connection,
	page: Option<String>,
	limit: usize,
) -> Result<(Option<String>, Vec<User>), UserFetchError> {
	let pager = PagedResults {
		size: limit as i32,
		cookie: page.map(|p| BASE64_URL_SAFE.decode(p))
			.unwrap_or(Ok(vec![]))
			.map_err(|_| UserFetchError::InvalidPage)?,
	};

	let filter = format!("({}=*)", CONFIG.users_ldap_id_field);
	let (results, status) = connection.with_controls(pager)
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

pub async fn get(
	connection: &mut Connection,
	uid: String,
) -> Result<User, UserFetchError> {
	let filter = format!("({}={})", CONFIG.users_ldap_id_field, ldap_escape(uid));
	let (results, _) = connection
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