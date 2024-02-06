use async_trait::async_trait;
use ldap3::{LdapConnAsync, LdapError, drive, Ldap, SearchEntry};

use deadpool::managed::{Manager, Metrics, RecycleResult};
use serde::Serialize;

use crate::config::CONFIG;

pub type Pool = deadpool::managed::Pool<LDAPConnectionManager>;
pub type Connection = deadpool::managed::Object<LDAPConnectionManager>;
pub struct LDAPConnectionManager(pub String);

// TODO: split this whole module into submodules

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
