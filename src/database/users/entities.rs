use ldap3::SearchEntry;
use serde::{Serialize, Deserialize};
use serde_with::skip_serializing_none;
use url::{Url, ParseError};

use crate::{config::CONFIG, permissions::Permission};

#[derive(Debug)]
pub enum TimestampParseError {
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

struct LDAPTimestamp {
	year: i32,
	month: u32,
	day: u32,
	hour: u32,
	minute: u32,
	second: u32,
}

impl LDAPTimestamp {
	pub fn unix_time(&self) -> Result<i64, TimestampParseError> {
		Ok(chrono::NaiveDate::from_ymd_opt(self.year, self.month, self.day)
			.ok_or(TimestampParseError::InvalidDate)?
			.and_hms_opt(self.hour, self.minute, self.second)
			.ok_or(TimestampParseError::InvalidTime)?
			.signed_duration_since(chrono::NaiveDateTime::UNIX_EPOCH)
			.num_seconds())
	}
}

impl TryFrom<&str> for LDAPTimestamp {
	type Error = TimestampParseError;

	fn try_from(value: &str) -> Result<Self, Self::Error> {
		if value.len() != 15 {
			return Err(TimestampParseError::IncorrectTimestampLength);
		}
		if &value[14..15] != "Z" {
			return Err(TimestampParseError::IncorrectTimestampSuffix);
		}

		let year = value[0..4].parse()
			.map_err(|_| TimestampParseError::IncorrectTimestampYear)?;
		let month = value[4..6].parse()
			.map_err(|_| TimestampParseError::IncorrectTimestampMonth)?;
		let day = value[6..8].parse()
			.map_err(|_| TimestampParseError::IncorrectTimestampDay)?;
		let hour = value[8..10].parse()
			.map_err(|_| TimestampParseError::IncorrectTimestampHour)?;
		let minute = value[10..12].parse()
			.map_err(|_| TimestampParseError::IncorrectTimestampMinute)?;
		let second = value[12..14].parse()
			.map_err(|_| TimestampParseError::IncorrectTimestampSecond)?;

		Ok(Self { year, month, day, hour, minute, second })
	}
}

#[derive(Debug)]
pub enum UserParseError {
	MissingId,
	MissingTimestamp,
	BadTimestamp(TimestampParseError),
}

#[derive(Debug, Serialize)]
pub struct User {
	#[serde(skip_serializing)]
	pub id: String,
	pub name: String,
	pub created_at: i64,
}

lazy_static! {
	static ref USER_FIELDS: [&'static str; 3] = [
		&CONFIG.ldap_users_id_field,
		&CONFIG.ldap_users_username_field,
		"createTimestamp"
	];
}

impl User {
	pub fn search_fields() -> [&'static str; 3] {
		*USER_FIELDS
	}
}

impl TryFrom<SearchEntry> for User {
	type Error = UserParseError;

	fn try_from(value: SearchEntry) -> Result<Self, Self::Error> {
		let id = value.attrs.get(&CONFIG.ldap_users_id_field)
			.and_then(|v| v.first())
			.ok_or(UserParseError::MissingId)?
			.to_owned();
		let name = value.attrs.get(&CONFIG.ldap_users_username_field)
			.and_then(|v| v.first())
			.ok_or(UserParseError::MissingId)?
			.to_owned();
		let created_at = value.attrs.get("createTimestamp")
			.and_then(|v| v.first())
			.ok_or(UserParseError::MissingTimestamp)
			.and_then(|s| {
				LDAPTimestamp::try_from(s.as_str())
					.and_then(|t| t.unix_time())
					.map_err(UserParseError::BadTimestamp)
			})?;

		Ok(User{ id, name, created_at })
	}
}

#[derive(Debug)]
pub enum RoleParseError {
	MissingName,
	InvalidIcon(ParseError),
}

#[skip_serializing_none]
#[derive(Debug, Serialize, Deserialize)]
pub struct Role {
	pub name: String,
	pub icon: Option<Url>,
	pub permissions: Vec<Permission>,
}


lazy_static! {
	static ref ROLE_FIELDS: [&'static str; 3] = [
		"cn",
		"pxlsspaceIcon",
		"pxlsspacePermission"
	];
}

impl Role {
	pub fn search_fields() -> [&'static str; 3] {
		*ROLE_FIELDS
	}
}

impl TryFrom<SearchEntry> for Role {
	type Error = RoleParseError;

	fn try_from(value: SearchEntry) -> Result<Self, Self::Error> {
		let name = value.attrs.get("cn")
			.and_then(|v| v.first())
			.ok_or(RoleParseError::MissingName)?
			.to_owned();
		let icon = value.attrs.get("pxlsspaceIcon")
			.and_then(|v| v.first())
			.map(|v| v.parse::<Url>())
			.transpose()
			.map_err(RoleParseError::InvalidIcon)?;
		let permissions = value.attrs.get("pxlsspacePermission")
			.cloned()
			.unwrap_or_default()
			.into_iter()
			// NOTE: silently drops invalid permissions
			.filter_map(|v| Permission::try_from(v.as_str()).ok())
			.collect();

		Ok(Role{ name, icon, permissions })
	}
}