use chrono::{Datelike, Timelike};
use ldap3::SearchEntry;
use serde::{Serialize, Deserialize};
use serde_with::skip_serializing_none;
use url::{Url, ParseError as UrlParseError};
use warp::http::Uri;

use crate::config::CONFIG;
use crate::filter::response::reference::Referenceable;
use crate::permissions::Permission;

#[derive(Debug)]
pub enum ParseError {
	User(UserParseError),
	Role(RoleParseError),
	Faction(FactionParseError),
}

impl From<UserParseError> for ParseError {
	fn from(value: UserParseError) -> Self {
		Self::User(value)
	}
}

impl From<RoleParseError> for ParseError {
	fn from(value: RoleParseError) -> Self {
		Self::Role(value)
	}
}

impl From<FactionParseError> for ParseError {
	fn from(value: FactionParseError) -> Self {
		Self::Faction(value)
	}
}

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

pub struct LDAPTimestamp {
	year: i32,
	month: u32,
	day: u32,
	hour: u32,
	minute: u32,
	second: u32,
}

impl From<i64> for LDAPTimestamp {
	fn from(value: i64) -> Self {
		let timestamp = chrono::DateTime::from_timestamp(value, 0).unwrap();
		Self {
			year: timestamp.year(),
			month: timestamp.month(),
			day: timestamp.day(),
			hour: timestamp.hour(),
			minute: timestamp.minute(),
			second: timestamp.second(),
		}
	}
}

impl From<LDAPTimestamp> for String {
	fn from(value: LDAPTimestamp) -> Self {
		let LDAPTimestamp { year, month, day, hour, minute, second } = value;
		format!(
			"{:04}{:02}{:02}{:02}{:02}{:02}Z",
			year, month, day, hour, minute, second,
		)
	}
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

#[derive(Debug, Serialize, Clone)]
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

impl From<&User> for Uri {
	fn from(user: &User) -> Self {
		format!("/users/{}", user.id).parse().unwrap()
	}
}

impl Referenceable for User {
	fn location(&self) -> Uri { Uri::from(self) }
}

#[derive(Debug)]
pub enum RoleParseError {
	MissingName,
	InvalidIcon(UrlParseError),
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

		Ok(Role { name, icon, permissions })
	}
}

impl From<&Role> for Uri {
	fn from(role: &Role) -> Self {
		format!("/roles/{}", role.name).parse().unwrap()
	}
}

impl Referenceable for Role {
	fn location(&self) -> Uri { Uri::from(self) }
}


#[derive(Debug)]
pub enum FactionParseError {
	MissingId,
	MissingName,
	InvalidIcon(UrlParseError),
	MissingTimestamp,
	BadTimestamp(TimestampParseError),
}

#[derive(Debug, Serialize)]
pub struct Faction {
	#[serde(skip_serializing)]
	pub id: String,
	pub name: String,
	pub created_at: i64,
	pub size: usize,
}

lazy_static! {
	static ref FACTION_FIELDS: [&'static str; 4] = [
		"cn",
		"pxlsspaceFactionName",
		//"pxlsspaceIcon",
		"createTimestamp",
		"member",
	];
}

impl Faction {
	pub fn search_fields() -> [&'static str; 4] {
		*FACTION_FIELDS
	}
}

impl TryFrom<SearchEntry> for Faction {
	type Error = FactionParseError;

	fn try_from(value: SearchEntry) -> Result<Self, Self::Error> {
		let id = value.attrs.get("cn")
			.and_then(|v| v.first())
			.ok_or(FactionParseError::MissingId)?
			.to_owned();
		let name = value.attrs.get("pxlsspaceFactionName")
			.and_then(|v| v.first())
			.ok_or(FactionParseError::MissingName)?
			.to_owned();
		//let icon = value.attrs.get("pxlsspaceIcon")
		//	.and_then(|v| v.first())
		//	.map(|v| v.parse::<Url>())
		//	.transpose()
		//	.map_err(FactionParseError::InvalidIcon)?;
		let created_at = value.attrs.get("createTimestamp")
			.and_then(|v| v.first())
			.ok_or(FactionParseError::MissingTimestamp)
			.and_then(|s| {
				LDAPTimestamp::try_from(s.as_str())
					.and_then(|t| t.unix_time())
					.map_err(FactionParseError::BadTimestamp)
			})?;
		let size = value.attrs.get("member")
			.map(|v| v.len())
			.unwrap_or(0);

		Ok(Faction { id, name, created_at, size })
	}
}

impl From<&Faction> for Uri {
	fn from(faction: &Faction) -> Self {
		format!("/factions/{}", faction.id).parse().unwrap()
	}
}

impl Referenceable for Faction {
	fn location(&self) -> Uri { Uri::from(self) }
}

#[derive(Debug, Serialize)]
pub struct JoinIntent {
	member: bool,
	faction: bool,
}

impl Default for JoinIntent {
	fn default() -> Self {
		Self { member: true, faction: true }
	}
}

#[derive(Debug, Serialize)]
pub struct FactionMember {
	#[serde(skip_serializing)]
	faction_id: String,
	#[serde(skip_serializing)]
	user_id: String,
	owner: bool,
	join_intent: JoinIntent,
}

impl FactionMember {
	pub fn new(faction_id: String, user_id: String, owner: bool) -> Self {
		let join_intent = JoinIntent::default();
		Self { owner, join_intent, faction_id, user_id }
	}
}

impl From<&FactionMember> for Uri {
	fn from(member: &FactionMember) -> Self {
		format!("/factions/{}/members/{}", member.faction_id, member.user_id).parse().unwrap()
	}
}

impl Referenceable for FactionMember {
	fn location(&self) -> Uri { Uri::from(self) }
}