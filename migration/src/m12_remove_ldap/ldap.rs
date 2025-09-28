mod config;

use std::collections::HashSet;

use ldap3::{Ldap, LdapError, LdapConnAsync, Scope, SearchEntry};
use chrono::{Datelike, Timelike};
use url::{Url, ParseError as UrlParseError};
use lazy_static::lazy_static;

use super::entities::{User, Role, RoleMembers, Faction, FactionMembers, FactionMember};

use config::CONFIG;

#[derive(Debug)]
pub enum Error {
	Ldap(LdapError),
	Parse(ParseError),
}

impl From<LdapError> for Error {
	fn from(value: LdapError) -> Self {
		Error::Ldap(value)
	}
}

impl<E: Into<ParseError>> From<E> for Error {
	fn from(value: E) -> Self {
		Error::Parse(value.into())
	}
}

pub struct Connection(Ldap);

impl Connection {
	pub async fn new() -> Result<Self, Error> {
		let (connection, mut ldap) = LdapConnAsync::new(CONFIG.ldap_url.as_str()).await?;
		ldap3::drive!(connection);
		let user = format!(
			"cn={},{}",
			CONFIG.ldap_manager_user.as_str(),
			CONFIG.ldap_base,
		);
		let password = CONFIG.ldap_manager_password.as_str();
		ldap.simple_bind(&user, password).await?.success()?;
		Ok(Self(ldap))
	}
	
	pub async fn load_users(&mut self) -> Result<Vec<User>, Error> {
		let (results, _) = self.0
			.search(
				&format!("ou={},{}", CONFIG.ldap_users_ou, CONFIG.ldap_base),
				Scope::OneLevel,
				&format!("({}=*)", CONFIG.ldap_users_id_field),
				User::search_fields(),
			).await?
			.success()?;

		results.into_iter()
			.map(SearchEntry::construct)
			.map(|user| User::try_from(user).map_err(Into::into))
			.collect()
	}
	
	pub async fn load_roles(&mut self) -> Result<Vec<Role>, Error> {
		let (results, _) = self.0
			.search(
				&format!("ou={},{}", CONFIG.ldap_roles_ou, CONFIG.ldap_base),
				Scope::OneLevel,
				"(cn=*)",
				Role::search_fields(),
			).await?
			.success()?;

		results.into_iter()
			.map(SearchEntry::construct)
			.map(|role| Role::try_from(role).map_err(Into::into))
			.collect()
	}
	
	pub async fn load_role_members(&mut self) -> Result<Vec<RoleMembers>, Error> {
		let (results, _) = self.0
			.search(
				&format!("ou={},{}", CONFIG.ldap_roles_ou, CONFIG.ldap_base),
				Scope::OneLevel,
				"(cn=*)",
				["cn", "member"],
			).await?
			.success()?;

		results.into_iter()
			.map(SearchEntry::construct)
			.map(|role| RoleMembers::try_from(role).map_err(Into::into))
			.collect()
	}
	
	pub async fn load_factions(&mut self) -> Result<Vec<Faction>, Error> {
		let (results, _) = self.0
			.search(
				&format!("ou={},{}", CONFIG.ldap_factions_ou, CONFIG.ldap_base),
				Scope::OneLevel,
				"(cn=*)",
				Faction::search_fields(),
			).await?
			.success()?;

		results.into_iter()
			.map(SearchEntry::construct)
			.map(|role| Faction::try_from(role).map_err(Into::into))
			.collect()
	}
	
	pub async fn load_faction_members(&mut self) -> Result<Vec<FactionMembers>, Error> {
		let (results, _) = self.0
			.search(
				&format!("ou={},{}", CONFIG.ldap_factions_ou, CONFIG.ldap_base),
				Scope::OneLevel,
				"(cn=*)",
				["cn", "member", "pxlsspaceFactionOwner"],
			).await?
			.success()?;

		results.into_iter()
			.map(SearchEntry::construct)
			.map(|role| FactionMembers::try_from(role).map_err(Into::into))
			.collect()
	}
}

fn extract_uid<S: AsRef<str>>(dn: S) -> Option<String> {
	let first_part = dn.as_ref().split(',').next()?;
	let (attr, value) = first_part.split_once('=')?;
	
	if attr == CONFIG.ldap_users_id_field {
		Some(value.to_string())
	} else {
		None
	}
}

#[derive(Debug)]
pub enum ParseError {
	User(UserParseError),
	Role(RoleParseError),
	RoleMember(RoleMemberParseError),
	Faction(FactionParseError),
	FactionMember(FactionMemberParseError),
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

impl From<RoleMemberParseError> for ParseError {
	fn from(value: RoleMemberParseError) -> Self {
		Self::RoleMember(value)
	}
}

impl From<FactionParseError> for ParseError {
	fn from(value: FactionParseError) -> Self {
		Self::Faction(value)
	}
}

impl From<FactionMemberParseError> for ParseError {
	fn from(value: FactionMemberParseError) -> Self {
		Self::FactionMember(value)
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

#[cfg(feature = "migrate-ldap")]
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
	MissingUsername,
	MissingTimestamp,
	BadTimestamp(TimestampParseError),
}

#[cfg(feature = "migrate-ldap")]
lazy_static! {
	static ref USER_FIELDS: [&'static str; 3] = [
		&CONFIG.ldap_users_id_field,
		&CONFIG.ldap_users_username_field,
		"createTimestamp"
	];
}

#[cfg(feature = "migrate-ldap")]
impl User {
	pub fn search_fields() -> [&'static str; 3] {
		*USER_FIELDS
	}

	pub fn id_from(entry: &SearchEntry) -> Result<String, UserParseError> {
		entry.attrs.get(&CONFIG.ldap_users_id_field)
			.and_then(|v| v.first())
			.map(String::to_owned)
			.ok_or(UserParseError::MissingId)
	}
}

#[cfg(feature = "migrate-ldap")]
impl TryFrom<SearchEntry> for User {
	type Error = UserParseError;

	fn try_from(value: SearchEntry) -> Result<Self, Self::Error> {
		let id = value.attrs.get(&CONFIG.ldap_users_id_field)
			.and_then(|v| v.first())
			.ok_or(UserParseError::MissingId)?
			.to_owned();
		let name = value.attrs.get(&CONFIG.ldap_users_username_field)
			.and_then(|v| v.first())
			.ok_or(UserParseError::MissingUsername)?
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
	InvalidIcon(UrlParseError),
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

#[cfg(feature = "migrate-ldap")]
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
			.collect();

		Ok(Role { name, icon, permissions })
	}
}

#[derive(Debug)]
pub enum RoleMemberParseError {
	MissingRole,
	InvalidUid,
}

#[cfg(feature = "migrate-ldap")]
impl TryFrom<SearchEntry> for RoleMembers {
	type Error = RoleMemberParseError;

	fn try_from(value: SearchEntry) -> Result<Self, Self::Error> {
		let role = value.attrs.get("cn")
			.and_then(|v| v.first())
			.ok_or(RoleMemberParseError::MissingRole)?
			.to_owned();
		let users = value.attrs.get("member")
			.map(|members| {
				members.iter()
					.map(extract_uid)
					.map(|o| o.ok_or(RoleMemberParseError::InvalidUid))
					.collect::<Result<_, _>>()
			})
			.unwrap_or(Ok(vec![]))?;

		Ok(RoleMembers { role, users })
	}
}

#[derive(Debug)]
pub enum FactionParseError {
	MissingId,
	MissingName,
	InvalidIcon(UrlParseError),
	MissingTimestamp,
	BadTimestamp(TimestampParseError),
}

lazy_static! {
	static ref FACTION_FIELDS: [&'static str; 4] = [
		"cn",
		"pxlsspaceFactionName",
		"pxlsspaceIcon",
		"createTimestamp",
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
		let cn = value.attrs.get("cn")
			.and_then(|v| v.first())
			.map(String::to_owned)
			.ok_or(FactionParseError::MissingId)?;
		let name = value.attrs.get("pxlsspaceFactionName")
			.and_then(|v| v.first())
			.ok_or(FactionParseError::MissingName)?
			.to_owned();
		let icon = value.attrs.get("pxlsspaceIcon")
			.and_then(|v| v.first())
			.map(|v| v.parse::<Url>())
			.transpose()
			.map_err(FactionParseError::InvalidIcon)?;
		let created_at = value.attrs.get("createTimestamp")
			.and_then(|v| v.first())
			.ok_or(FactionParseError::MissingTimestamp)
			.and_then(|s| {
				LDAPTimestamp::try_from(s.as_str())
					.and_then(|t| t.unix_time())
					.map_err(FactionParseError::BadTimestamp)
			})?;

		Ok(Faction { cn, name, created_at, icon })
	}
}

#[derive(Debug)]
pub enum FactionMemberParseError {
	MissingFaction,
	InvalidUid,
}

impl TryFrom<SearchEntry> for FactionMembers {
	type Error = FactionMemberParseError;

	fn try_from(value: SearchEntry) -> Result<Self, Self::Error> {
		let faction = value.attrs.get("cn")
			.and_then(|v| v.first())
			.ok_or(FactionMemberParseError::MissingFaction)?
			.to_owned();
		
		let owners = value.attrs.get("pxlsspaceFactionOwner")
			.map(|members| {
				members.iter()
					.map(extract_uid)
					.map(|o| o.ok_or(FactionMemberParseError::InvalidUid))
					.collect::<Result<_, _>>()
			})
			.unwrap_or(Ok(HashSet::new()))?;
		
		let users = value.attrs.get("member")
			.map(|members| {
				members.iter().map(|dn| {
					let user = extract_uid(dn).ok_or(FactionMemberParseError::InvalidUid)?;
					let owner = owners.contains(&user);
					Ok(FactionMember { user, owner })
				}).collect::<Result<_, _>>()
			})
			.unwrap_or(Ok(vec![]))?;

		Ok(FactionMembers { faction, users })
	}
}
