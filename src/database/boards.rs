use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use std::fmt;

use bytes::{BytesMut, BufMut};
use enumset::EnumSet;
use reqwest::StatusCode;
use sea_orm::sea_query::{Expr, SimpleExpr};
use sea_orm::{AccessMode, IsolationLevel, TryInsertResult, Value};
use sea_orm::{ActiveValue::NotSet, ColumnTrait, ConnectOptions, ConnectionTrait, Database, DatabaseConnection, DatabaseTransaction, DbErr, EntityTrait, FromQueryResult, Iden, ModelTrait, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect, QueryTrait, RelationTrait, Set, SqlErr, StreamTrait, TransactionTrait};
use sea_orm_migration::MigratorTrait;
use sea_query::{ColumnRef, IntoIden};
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;
use tokio::sync::{Mutex, RwLock};
use tokio_stream::StreamExt;
use url::form_urlencoded::byte_serialize;
use url::Url;
use warp::reject::Reject;
use warp::reply::Reply;
use warp::http::Uri;

use crate::config::CONFIG;
use crate::board::{CooldownCache, PendingPlacement};
use crate::filter::response::paginated_list::{Page, PageToken};
use crate::filter::response::reference::{Referencable, Reference};
use crate::permissions::Permission;
use crate::routes::factions::factions::members::FactionMemberFilter;
use crate::routes::factions::factions::FactionFilter;
use crate::routes::roles::roles::RoleFilter;
use crate::routes::site_notices::notices::{Notice, NoticeFilter};
use crate::routes::board_notices::boards::notices::{BoardsNoticePageToken, BoardsNotice, BoardNoticeFilter};
use crate::routes::reports::reports::{ReportPageToken, ReportFilter, Report, ReportStatus, Artifact};
use crate::routes::core::boards::pixels::PlacementFilter;
use crate::routes::placement_statistics::users::PlacementColorStatistics;
use crate::routes::user_bans::users::BanFilter;
use crate::routes::users::users::UserFilter;
use crate::board::{ActivityCache, Palette, Color, Board, Placement, PlacementPageToken, Sector, LastPlacement, CachedPlacement, WriteBuffer};
use crate::routes::site_notices::notices::NoticePageToken;

mod entities;

use entities::*;
use migration::Migrator;

use super::Order;

#[derive(Debug, Eq, PartialEq, Clone, Copy, Hash)]
pub struct BanSpecifier(i32);

impl FromStr for BanSpecifier {
	type Err = std::num::ParseIntError;
	fn from_str(s: &str) -> Result<Self, Self::Err> {
		s.parse().map(Self)
	}
}

impl From<&BanSpecifier> for Value {
	fn from(value: &BanSpecifier) -> Self {
		Value::from(value.0)
	}
}

impl From<BanSpecifier> for Value {
	fn from(value: BanSpecifier) -> Self {
		Value::from(&value)
	}
}

impl fmt::Display for BanSpecifier {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{}", self.0)
	}
}

#[derive(Debug, Serialize, Clone)]
pub struct Ban {
	#[serde(skip_serializing)]
	id: i32,
	#[serde(skip_serializing)]
	user: User,
	pub created_at: u64,
	pub expires_at: Option<u64>,
	pub issuer: Option<Reference<User>>,
	pub reason: Option<String>,
}

impl Ban {
	pub fn specifier(&self) -> BanSpecifier {
		BanSpecifier(self.id)
	}
}

impl Referencable for Ban {
	fn uri(&self) -> Uri {
		format!("/users/{}/bans/{}", self.user.specifier(), self.id).parse().unwrap()
	}
}

#[derive(Debug, Default, Deserialize)]
pub struct BanPageToken(pub u32);

impl PageToken for BanPageToken {}

impl fmt::Display for BanPageToken {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{}", self.0)
	}
}

#[derive(Iden)]
enum BanUser {
	Table,
	Subject,
	Name,
	CreatedAt,
}

#[derive(Iden)]
enum BanIssuer {
	Table,
	Subject,
	Name,
	CreatedAt,
}

#[derive(FromQueryResult)]
struct BanFull {
	id: i32,
	created_at: i64,
	expires_at: Option<i64>,
	reason: Option<String>,
	
	user_id: i32,
	user_subject: String,
	user_name: String,
	user_created_at: i64,
	
	issuer: Option<i32>,
	issuer_subject: Option<String>,
	issuer_name: Option<String>,
	issuer_created_at: Option<i64>,
}

impl BanFull {
	fn split(self) -> (ban::Model, user::Model, Option<user::Model>) {
		let ban = ban::Model {
			id: self.id,
			created_at: self.created_at,
			expires_at: self.expires_at,
			reason: self.reason,
			user_id: self.user_id,
			issuer: self.issuer,
		};
		
		let user = user::Model {
			id: self.user_id,
			subject: self.user_subject,
			name: self.user_name,
			created_at: self.user_created_at,
		};
		
		let issuer = self.issuer.map(|id| user::Model {
			id,
			subject: self.issuer_subject.unwrap(),
			name: self.issuer_name.unwrap(),
			created_at: self.issuer_created_at.unwrap(),
		});
		
		(ban, user, issuer)
	}
}

#[derive(Debug, Eq, PartialEq, Clone, Copy, Hash)]
pub struct UserSpecifier(i32);

impl UserSpecifier {
	pub fn null() -> Self { Self(0) }
}

impl FromStr for UserSpecifier {
	type Err = std::num::ParseIntError;
	fn from_str(s: &str) -> Result<Self, Self::Err> {
		s.parse().map(Self)
	}
}

impl From<&UserSpecifier> for Value {
	fn from(value: &UserSpecifier) -> Self {
		Value::from(value.0)
	}
}

impl From<UserSpecifier> for Value {
	fn from(value: UserSpecifier) -> Self {
		Value::from(&value)
	}
}

impl<'de> Deserialize<'de> for UserSpecifier {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where D: serde::Deserializer<'de> {
		struct V;

		impl<'de> serde::de::Visitor<'de> for V {
			type Value = UserSpecifier;
			
			fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
				write!(f, "A user uri reference")
			}

			fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
			where E: serde::de::Error, {
				let uri = v.parse::<Uri>().map_err(E::custom)?;
				// TODO: maybe domain/scheme validation
				let mut segments = uri.path().split('/');

				if !matches!(segments.next(), Some("")) {
					return Err(E::custom("expected absolute path"))
				}
				
				if !matches!(segments.next(), Some("users")) {
					return Err(E::custom("expected /users/"))
				}
				
				let id = match segments.next() {
					Some(id) => id.parse().map_err(|_| E::custom("user id should be a number"))?,
					None => return Err(E::custom("expected user id")),
				};
				
				if let Some(unexpected) = segments.next() {
					let error = format!("unexpected path segment \"{}\"", unexpected);
					return Err(E::custom(error));
				}

				Ok(UserSpecifier(id))
			}
		}

		deserializer.deserialize_str(V)
	}
}

impl fmt::Display for UserSpecifier {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{}", self.0)
	}
}

#[derive(Debug, Serialize, Clone)]
pub struct User {
	#[serde(skip_serializing)]
	id: i32,
	#[serde(skip_serializing)]
	pub subject: String,
	pub name: String,
	pub created_at: i64,
}

impl User {
	pub fn specifier(&self) -> UserSpecifier {
		UserSpecifier(self.id)
	}
}

impl Referencable for User {
	fn uri(&self) -> Uri {
		format!("/users/{}", self.specifier()).parse().unwrap()
	}
}

impl From<user::Model> for User {
	fn from(user: user::Model) -> Self {
		let user::Model { id, subject, name, created_at } = user;
		User { id, subject, name, created_at }
	}
}

#[derive(Debug, Default, Deserialize)]
pub struct UsersPageToken(pub u32);
impl PageToken for UsersPageToken {}
impl From<&user::Model> for UsersPageToken {
	fn from(value: &user::Model) -> Self {
		Self(value.id as _)
	}
}
impl fmt::Display for UsersPageToken {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{}", self.0)
	}
}

#[derive(Debug, Default, Deserialize)]
pub struct UserRolesPageToken(pub u32);
impl PageToken for UserRolesPageToken {}
impl From<&role::Model> for UserRolesPageToken {
	fn from(value: &role::Model) -> Self {
		Self(value.id as _)
	}
}
impl fmt::Display for UserRolesPageToken {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{}", self.0)
	}
}

#[derive(Debug, Clone, Serialize)]
pub struct UserFactionMember {
	faction: Reference<Faction>,
	member: Reference<FactionMember>,
}

impl UserFactionMember {
	fn user(&self) -> &User {
		&self.member.view.user.view
	}
}

impl From<FactionMember> for UserFactionMember {
	fn from(value: FactionMember) -> Self {
		Self {
			faction: Reference::from(value.faction.clone()),
			member: Reference::from(value),
		}
	}
}

#[derive(Debug, Default, Deserialize)]
pub struct UserFactionsPageToken(pub u32);
impl PageToken for UserFactionsPageToken {}
impl From<&FactionMemberFull> for UserFactionsPageToken {
	fn from(value: &FactionMemberFull) -> Self {
		Self(value.faction_id as _)
	}
}
impl fmt::Display for UserFactionsPageToken {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{}", self.0)
	}
}

#[derive(Debug)]
pub struct RoleMember {
	user: Reference<User>,
	role: Reference<Role>,
}

#[derive(Debug, Eq, PartialEq, Clone, Copy, Hash)]
pub struct RoleSpecifier(i32);

impl FromStr for RoleSpecifier {
	type Err = std::num::ParseIntError;
	fn from_str(s: &str) -> Result<Self, Self::Err> {
		s.parse().map(Self)
	}
}

impl From<&RoleSpecifier> for Value {
	fn from(value: &RoleSpecifier) -> Self {
		Value::from(value.0)
	}
}

impl From<RoleSpecifier> for Value {
	fn from(value: RoleSpecifier) -> Self {
		Value::from(&value)
	}
}

impl<'de> Deserialize<'de> for RoleSpecifier {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where D: serde::Deserializer<'de> {
		struct V;

		impl<'de> serde::de::Visitor<'de> for V {
			type Value = RoleSpecifier;
			
			fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
				write!(f, "A role uri reference")
			}

			fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
			where E: serde::de::Error, {
				let uri = v.parse::<Uri>().map_err(E::custom)?;
				// TODO: maybe domain/scheme validation
				let mut segments = uri.path().split('/');

				if !matches!(segments.next(), Some("")) {
					return Err(E::custom("expected absolute path"))
				}
				
				if !matches!(segments.next(), Some("roles")) {
					return Err(E::custom("expected /roles/"))
				}
				
				let id = match segments.next() {
					Some(id) => id.parse().map_err(|_| E::custom("role id should be a number"))?,
					None => return Err(E::custom("expected role id")),
				};
				
				if let Some(unexpected) = segments.next() {
					let error = format!("unexpected path segment \"{}\"", unexpected);
					return Err(E::custom(error));
				}

				Ok(RoleSpecifier(id))
			}
		}

		deserializer.deserialize_str(V)
	}
}

impl fmt::Display for RoleSpecifier {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{}", self.0)
	}
}

#[skip_serializing_none]
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Role {
	#[serde(skip_serializing)]
	id: i32,
	pub name: String,
	pub icon: Option<Url>,
	pub permissions: Vec<Permission>,
}

impl Role {
	fn specifier(&self) -> RoleSpecifier {
		RoleSpecifier(self.id)
	}
}

impl Referencable for Role {
	fn uri(&self) -> Uri {
		format!("/roles/{}", self.specifier()).parse().unwrap()
	}
}

impl From<role::Model> for Role {
	fn from(role: role::Model) -> Self {
		let role::Model { id, name, icon, permissions } = role;
		// silently drops invalid icon urls
		let icon = icon.and_then(|icon| icon.parse().ok());
		// silently drops invalid permissions
		let permissions = permissions.split(',')
			.map(str::trim)
			.map(Permission::try_from)
			.filter_map(Result::ok)
			.collect::<EnumSet<_>>()
			.into_iter()
			.collect();
		Role { id, name, icon, permissions }
	}
}

#[derive(Debug, Default, Deserialize)]
pub struct RolesPageToken(pub u32);
impl PageToken for RolesPageToken {}
impl From<&role::Model> for RolesPageToken {
	fn from(value: &role::Model) -> Self {
		Self(value.id as _)
	}
}
impl fmt::Display for RolesPageToken {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{}", self.0)
	}
}

#[derive(Debug, FromQueryResult)]
pub struct FactionFull {
	id: i32,
	name: String,
	icon: Option<String>,
	created_at: i64,
	size: i64,
}

impl FactionFull {
	fn from_model_and_size(model: faction::Model, size: i64) -> Self {
		let faction::Model { id, name, icon, created_at } = model;
		FactionFull { id, name, icon, created_at, size }
	}
}

#[derive(Debug, Clone, Copy)]
pub struct FactionSpecifier(i32);

impl From<&FactionSpecifier> for Uri {
	fn from(value: &FactionSpecifier) -> Self {
		format!("/factions/{}", value.0).parse().unwrap()
	}
}

impl From<FactionSpecifier> for Uri {
	fn from(value: FactionSpecifier) -> Self {
		Uri::from(&value)
	}
}

impl From<&FactionSpecifier> for Value {
	fn from(value: &FactionSpecifier) -> Self {
		Value::from(value.0)
	}
}

impl From<FactionSpecifier> for Value {
	fn from(value: FactionSpecifier) -> Self {
		Value::from(&value)
	}
}

impl FromStr for FactionSpecifier {
	type Err = std::num::ParseIntError;
	fn from_str(s: &str) -> Result<Self, Self::Err> {
		s.parse().map(Self)
	}
}

impl fmt::Display for FactionSpecifier {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{}", self.0)
	}
}

#[derive(Debug, Clone, Serialize)]
pub struct Faction {
	#[serde(skip_serializing)]
	id: i32,
	pub name: String,
	pub icon: Option<Url>,
	pub created_at: i64,
	pub size: usize,
}

impl Faction {
	pub fn specifier(&self) -> FactionSpecifier {
		FactionSpecifier(self.id)
	}
}

impl Referencable for Faction {
	fn uri(&self) -> Uri {
		self.specifier().into()
	}
}

impl From<FactionFull> for Faction {
	fn from(faction: FactionFull) -> Self {
		let FactionFull { id, name, icon, created_at, size } = faction;
		// silently drops invalid icon urls
		let icon = icon.and_then(|i| i.parse().ok());
		let size = size as usize;
		Faction { id, name, icon, created_at, size }
	}
}

#[derive(Debug, Default, Deserialize)]
pub struct FactionsPageToken(pub u32);
impl PageToken for FactionsPageToken {}
impl From<&FactionFull> for FactionsPageToken {
	fn from(value: &FactionFull) -> Self {
		Self(value.id as _)
	}
}
impl fmt::Display for FactionsPageToken {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{}", self.0)
	}
}

#[derive(Clone, Debug, Serialize)]
pub struct JoinIntent {
	pub member: bool,
	pub faction: bool,
}

#[derive(Debug, FromQueryResult)]
pub struct FactionMemberFull {
	invited: bool,
	imposed: bool,
	owner: bool,
	
	faction_id: i32,
	faction_name: String,
	faction_icon: Option<String>,
	faction_created_at: i64,
	faction_size: i64,

	member_id: i32,
	member_subject: String,
	member_name: String,
	member_created_at: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct FactionMember {
	#[serde(skip_serializing)]
	faction: Faction,
	user: Reference<User>,
	join_intent: JoinIntent,
	owner: bool,
}

impl FactionMember {
	pub fn faction(&self) -> Reference<Faction> {
		Reference::from(self.faction.clone())
	}
	
	fn from_parts(
		meta: faction_member::Model,
		faction: Faction,
		member: User,
	) -> Self {
		Self {
			user: Reference::from(member),
			faction,
			join_intent: JoinIntent {
				member: meta.imposed,
				faction: meta.invited,
			},
			owner: meta.owner,
		}
	}
}

impl Referencable for FactionMember {
	fn uri(&self) -> Uri {
		let fid = self.faction.id;
		let uid = self.user.view.id;
		
		format!("/factions/{fid}/members/{uid}").parse().unwrap()
	}
}

impl From<FactionMemberFull> for FactionMember {
	fn from(faction_member: FactionMemberFull) -> Self {
		let join_intent = JoinIntent {
			faction: faction_member.invited,
			member: faction_member.imposed,
		};
		let owner = faction_member.owner;
		
		let faction = FactionFull {
			id: faction_member.faction_id,
			name: faction_member.faction_name,
			icon: faction_member.faction_icon,
			created_at: faction_member.faction_created_at,
			size: faction_member.faction_size,
		}.into();
		
		let user = Reference::from(User::from(user::Model {
			id: faction_member.member_id,
			name: faction_member.member_name,
			subject: faction_member.member_subject,
			created_at: faction_member.member_created_at,
		}));
		
		FactionMember { faction, user, join_intent, owner }
	}
}

#[derive(Debug, Default, Deserialize)]
pub struct FactionMembersPageToken(pub u32);
impl PageToken for FactionMembersPageToken {}
impl From<&FactionMemberFull> for FactionMembersPageToken {
	fn from(value: &FactionMemberFull) -> Self {
		Self(value.member_id as _)
	}
}
impl fmt::Display for FactionMembersPageToken {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{}", self.0)
	}
}


#[derive(Debug)]
pub enum DatabaseError {
	DbErr(sea_orm::DbErr),
}

impl From<sea_orm::DbErr> for DatabaseError {
	fn from(value: sea_orm::DbErr) -> Self {
		DatabaseError::DbErr(value)
	}
}

impl From<&DatabaseError> for StatusCode {
	fn from(error: &DatabaseError) -> Self {
		match error {
			DatabaseError::DbErr(err) => {
				eprintln!("{err:?}");
				StatusCode::INTERNAL_SERVER_ERROR
			}
		}
	}
}

impl From<DatabaseError> for StatusCode {
	fn from(error: DatabaseError) -> Self {
		StatusCode::from(&error)
	}
}

impl Reply for DatabaseError {
	fn into_response(self) -> warp::reply::Response {
		StatusCode::from(&self).into_response()
	}
}

impl Reject for DatabaseError {}

type DbResult<T> = Result<T, DatabaseError>;

pub struct BoardsDatabase {
	pool: DatabaseConnection,
}

#[async_trait::async_trait]
impl super::Database for BoardsDatabase {
	type Error = DbErr;
	type Connection = BoardsConnection<DatabaseConnection>;

	async fn connect() -> Result<Self, Self::Error> {
		let url = CONFIG.database_url.to_string();
		let mut connect_options = ConnectOptions::new(url);
		connect_options
			.connect_timeout(Duration::from_secs(2))
			.acquire_timeout(Duration::from_secs(2));
		
		let pool = Database::connect(connect_options).await?;
		Migrator::up(&pool, None).await?;
		Ok(Self { pool })
	}

	async fn connection(&self) -> Result<Self::Connection, Self::Error> {
		let connection = self.pool.clone();
		Ok(BoardsConnection { connection })
	}
}

#[derive(Default, Debug, Clone, Copy)]
enum BanStatus {
	#[default]
	NotBanned,
	BannedUntil(u64),
	Permabanned,
}

impl BanStatus {
	async fn from_db<Connection: TransactionTrait + ConnectionTrait + StreamTrait>(
		user: &UserSpecifier,
		connection: &Connection
	) -> DbResult<Self> {
		let permanent_ban_count = ban::Entity::find()
			.filter(ban::Column::UserId.eq(user))
			.filter(ban::Column::ExpiresAt.is_null())
			.count(connection).await?;
		let largest_expiry_query = ban::Entity::find()
			.select_only()
			.column_as(ban::Column::ExpiresAt.max(), "expiry")
			.filter(ban::Column::UserId.eq(user))
			.build(connection.get_database_backend());
		let largest_expiry = connection.query_one(largest_expiry_query).await?
			.unwrap() // max always returns one row and will be null if there were no rows
			.try_get::<Option<i64>>("", "expiry")?;

		if permanent_ban_count > 0 {
			Ok(Self::Permabanned)
		} else if let Some(expiry) = largest_expiry {
			Ok(Self::BannedUntil(expiry as _))
		} else {
			Ok(Self::NotBanned)
		}
	}
}
#[derive(Default)]
struct BansCache {
	bans: RwLock<HashMap<UserSpecifier, BanStatus>>,
}

impl BansCache {
	async fn check<Connection: TransactionTrait + ConnectionTrait + StreamTrait>(
		&self,
		user: &UserSpecifier,
		connection: &Connection
	) -> DbResult<BanStatus> {
		let bans = self.bans.read().await;
		if let Some(&ban) = bans.get(user) {
			Ok(ban)
		} else {
			drop(bans);
			let mut bans = self.bans.write().await;
			let status = BanStatus::from_db(user, connection).await?;
			bans.insert(*user, status);
			Ok(status)
		}
	}

	async fn invalidate(&self, user: &UserSpecifier) {
		self.bans.write().await.remove(user);
	}
}

lazy_static! {
	// static ref USER_ID_CACHE: UserIdCache = UserIdCache::default();
	static ref BANS_CACHE: BansCache = BansCache::default();
}

pub struct BoardsConnection<Connection: TransactionTrait + ConnectionTrait + StreamTrait> {
	connection: Connection,
}

impl BoardsConnection<DatabaseTransaction> {
	pub async fn commit(self) -> DbResult<()> {
		self.connection.commit().await
			.map_err(DatabaseError::from)
	}
}

impl<C: TransactionTrait + ConnectionTrait + StreamTrait> BoardsConnection<C> {
	pub async fn begin(&self) -> DbResult<BoardsConnection<DatabaseTransaction>> {
		self.connection.begin().await
			.map(|connection| BoardsConnection { connection })
			.map_err(DatabaseError::from)
	}
	
	pub async fn begin_with_config(
		&self,
		isolation_level: Option<IsolationLevel>,
		access_mode: Option<AccessMode>,
	) -> DbResult<BoardsConnection<DatabaseTransaction>> {
		self.connection.begin_with_config(isolation_level, access_mode).await
			.map(|connection| BoardsConnection { connection })
			.map_err(DatabaseError::from)
	}

	// pub async fn get_uid(&self, user_id: &str) -> Result<i32, BoardsDatabaseError> {
	// 	USER_ID_CACHE.get_id(user_id.to_owned(), &self.connection).await
	// }

	pub async fn list_boards(
		&self,
		pool: Arc<BoardsDatabase>,
	) -> DbResult<Vec<Board>> {
		let db_boards = board::Entity::find()
			.all(&self.connection).await?;

		let mut boards = Vec::with_capacity(db_boards.len());

		for board in db_boards {
			boards.push(self.board_from_model(board, Arc::clone(&pool)).await?);
		}

		Ok(boards)
	}

	async fn board_from_model(
		&self,
		board: board::Model,
		pool: Arc<BoardsDatabase>,
	) -> DbResult<Board> {
		let id = board.id;

		let transaction = self.connection.begin().await?;

		let palette: Palette = board.find_related(color::Entity)
			.all(&transaction).await?
			.into_iter()
			.map(|color| {
				let index = color.index as u32;
				let color = Color {
					name: color.name,
					value: color.value as u32,
					system_only: color.system_only,
				};

				(index, color)
			})
			.collect();

		let stats_query = placement::Entity::find()
			.select_only()
			.column(placement::Column::UserId)
			.column(placement::Column::Color)
			.column_as(placement::Column::Timestamp.count(), "count")
			.group_by(placement::Column::UserId)
			.group_by(placement::Column::Color)
			.filter(placement::Column::Board.eq(id))
			.build(transaction.get_database_backend());
		let stats = transaction.query_all(stats_query).await?;

		let user_field = placement::Column::UserId.to_string();
		let color_field = placement::Column::Color.to_string();

		let mut stats_by_user = HashMap::<_, PlacementColorStatistics>::new();

		for stat in stats {
			let user = stat.try_get::<i32>("", &user_field).unwrap();
			let user_stats = stats_by_user.entry(UserSpecifier(user)).or_default();

			let color = stat.try_get::<i16>("", &color_field).unwrap() as _;
			let placed = stat.try_get::<i64>("", "count").unwrap() as usize;

			user_stats.colors.entry(color).or_default().placed += placed;
		}

		let statistics_cache = stats_by_user.into();
		
		// TODO: make configurable
		const IDLE_TIMEOUT: u32 = 5 * 60;
		
		let unix_time = SystemTime::now()
			.duration_since(SystemTime::UNIX_EPOCH).unwrap()
			.as_secs();
		let timestamp: u32 = unix_time.saturating_sub(board.created_at as u64).max(1)
 			.try_into().unwrap();
		let epoch = SystemTime::now() - Duration::from_secs(timestamp as u64);
		
		// the point after which activity from users will be considered currently active
		let idle_begin = timestamp - IDLE_TIMEOUT;
		
		let max_stack_cooldown = board.max_stacked as u32 * CONFIG.cooldown;
		// the point after which users may possibly not have a full stack of pixels
		let cooldown_begin = timestamp - max_stack_cooldown;

		let mut activity_cache = ActivityCache::new(IDLE_TIMEOUT);
		let mut cooldown_cache = CooldownCache::new(board.max_stacked as u32, epoch);
		
		let placements = placement::Entity::find()
			.filter(placement::Column::Board.eq(id))
			.filter(placement::Column::Timestamp.gt(cooldown_begin)
				.or(placement::Column::Timestamp.gt(idle_begin)))
			.order_by(placement::Column::Timestamp, sea_orm::Order::Desc)
			.order_by(placement::Column::Id, sea_orm::Order::Desc)
			.all(&transaction).await?
			.into_iter()
			.map(|placement| CachedPlacement {
				modified: placement.timestamp as u32,
				position: placement.position as u64,
				user: UserSpecifier(placement.user_id),
			})
			.rev();
		
		for placement in placements {
			// TODO
			let density = 0;
			let timestamp = placement.modified;
			
			activity_cache.insert(timestamp, placement.user);
			let activity = activity_cache.count(timestamp) as u32;
			
			cooldown_cache.insert(timestamp, placement.user, activity, density);
		}
		
		let activity_cache = Mutex::new(activity_cache);
		let cooldown_cache = RwLock::new(cooldown_cache);
		
		transaction.commit().await?;

		Ok(Board::new(
			id,
			board.name,
			board.created_at as u64,
			serde_json::from_value(board.shape).unwrap(),
			palette,
			board.max_stacked as u32,
			statistics_cache,
			activity_cache,
			cooldown_cache,
			pool,
		))
	}

	pub async fn create_board(
		&self,
		name: String,
		shape: Vec<Vec<usize>>,
		palette: Palette,
		max_pixels_available: u32,
		pool: Arc<BoardsDatabase>,
	) -> DbResult<Board> {
		let now = SystemTime::now()
			.duration_since(UNIX_EPOCH)
			.unwrap()
			.as_secs();

		let transaction = self.begin().await?;

		let new_board = board::Entity::insert(board::ActiveModel {
				id: NotSet,
				name: Set(name),
				created_at: Set(now as i64),
				shape: Set(serde_json::to_value(shape).unwrap()),
				max_stacked: Set(max_pixels_available as i32),
			})
			.exec_with_returning(&transaction.connection).await?;

		transaction.replace_palette(palette, new_board.id).await?;
		transaction.commit().await?;
		
		let board = self.board_from_model(new_board, pool).await?;

		Ok(board)
	}

	async fn replace_palette(
		&self,
		palette: Palette,
		board_id: i32,
	) -> DbResult<()> {
		let transaction = self.begin().await?;

		color::Entity::delete_many()
			.filter(color::Column::Board.eq(board_id))
			.exec(&transaction.connection).await?;
	
		for (index, Color { name, value, system_only }) in palette {
			let color = color::ActiveModel {
				board: Set(board_id),
				index: Set(index as i32),
				name: Set(name.clone()),
				value: Set(value as i32),
				system_only: Set(system_only),
			};
	
			color::Entity::insert(color)
				.exec(&transaction.connection).await?;
		}
		
		match transaction.commit().await {
			Err(DatabaseError::DbErr(err)) => {
				if let Some(SqlErr::ForeignKeyConstraintViolation(_)) = err.sql_err() {
					// TODO: This is a user error (the new palette removes
					// colors which are currently used). It should either be
					// passed back up from here, or detected earlier and this
					// is essential asserted as unreachable.
					// Consequently, it needs to return 409 or something.
					Err(DatabaseError::DbErr(err))
				} else {
					Err(DatabaseError::DbErr(err))
				}
			},
			other => other,
		}
	}

	pub async fn update_board_info(
		&self,
		board_id: i32,
		name: Option<String>,
		shape: Option<Vec<Vec<usize>>>,
		palette: Option<Palette>,
		max_pixels_available: Option<u32>,
	) -> DbResult<()> {
		let transaction = self.begin().await?;
		
		if let Some(ref name) = name {
			board::Entity::update_many()
				.col_expr(board::Column::Name, name.into())
				.filter(board::Column::Id.eq(board_id))
				.exec(&transaction.connection).await?;
		}

		if let Some(palette) = palette {
			transaction.replace_palette(palette, board_id).await?;
		}

		if let Some(ref shape) = shape {
			board::Entity::update_many()
				.col_expr(board::Column::Shape, serde_json::to_value(shape).unwrap().into())
				.filter(board::Column::Id.eq(board_id))
				.exec(&transaction.connection).await?;

			// TODO: try and preserve data.
			board_sector::Entity::delete_many()
				.filter(board_sector::Column::Board.eq(board_id))
				.exec(&transaction.connection).await?;
		}

		if let Some(max_stacked) = max_pixels_available {
			board::Entity::update_many()
				.col_expr(board::Column::MaxStacked, (max_stacked as i32).into())
				.filter(board::Column::Id.eq(board_id))
				.exec(&transaction.connection).await?;
		}
		
		transaction.commit().await?;

		Ok(())
	}
	
	pub async fn delete_board(&self, board_id: i32) -> DbResult<()> {
		let transaction = self.begin().await?;

		board_sector::Entity::delete_many()
			.filter(board_sector::Column::Board.eq(board_id))
			.exec(&transaction.connection).await?;

		placement::Entity::delete_many()
			.filter(placement::Column::Board.eq(board_id))
			.exec(&transaction.connection).await?;

		color::Entity::delete_many()
			.filter(color::Column::Board.eq(board_id))
			.exec(&transaction.connection).await?;

		board::Entity::delete_many()
			.filter(board::Column::Id.eq(board_id))
			.exec(&transaction.connection).await?;
		
		transaction.commit().await?;

		Ok(())
	}

	pub async fn last_place_time(
		&self,
		board_id: i32,
		user: &UserSpecifier,
	) -> DbResult<u32> {
		placement::Entity::find()
			.filter(
				placement::Column::Board.eq(board_id)
					.and(placement::Column::UserId.eq(user)),
			)
			.order_by(placement::Column::Timestamp, sea_orm::Order::Desc)
			.order_by(placement::Column::Id, sea_orm::Order::Desc)
			.one(&self.connection).await
			.map(|option| option.map(|placement| placement.timestamp))
			.map(|timestamp| timestamp.unwrap_or(0) as u32)
			.map_err(DatabaseError::from)
	}

	pub async fn list_placements(
		&self,
		board_id: i32,
		token: PlacementPageToken,
		limit: usize,
		order: Order,
		filter: PlacementFilter,
	) -> DbResult<Page<Placement>> {
		let column_timestamp_id_pair = Expr::tuple([
			Expr::col(placement::Column::Timestamp).into(),
			Expr::col(placement::Column::Id).into(),
		]);

		let value_timestamp_id_pair = Expr::tuple([
			(token.timestamp as i32).into(),
			(token.id as i32).into(),
		]);

		let compare_lhs = column_timestamp_id_pair.clone();
		let compare_rhs = value_timestamp_id_pair;
		let compare = match order {
			Order::Forward => Expr::gt(compare_lhs, compare_rhs),
			Order::Reverse => Expr::lt(compare_lhs, compare_rhs),
		};

		let order = match order {
			Order::Forward => sea_orm::Order::Asc,
			Order::Reverse => sea_orm::Order::Desc,
		};

		let placements = placement::Entity::find()
			.find_also_related(user::Entity)
			.filter(placement::Column::Board.eq(board_id))
			.filter(compare)
			.apply_if(filter.color.start, |q, start| q.filter(placement::Column::Color.gte(start)))
			.apply_if(filter.color.end, |q, end| q.filter(placement::Column::Color.lte(end)))
			.apply_if(filter.user.as_ref(), |q, id| q.filter(placement::Column::UserId.eq(id)))
			.apply_if(filter.position.start, |q, start| q.filter(placement::Column::Position.gte(start)))
			.apply_if(filter.position.end, |q, end| q.filter(placement::Column::Position.lte(end)))
			.apply_if(filter.timestamp.start, |q, start| q.filter(placement::Column::Timestamp.gte(start)))
			.apply_if(filter.timestamp.end, |q, end| q.filter(placement::Column::Timestamp.lte(end)))
			.order_by(column_timestamp_id_pair, order)
			.limit(limit as u64 + 1) // fetch one extra to see if this is the end of the data
			.all(&self.connection).await?;


		let next = placements.windows(2).nth(limit.saturating_sub(1))
			.map(|pair| &pair[0]) // we have [last, next] and want the data for last
			.map(|(placement, _)| PlacementPageToken {
				id: placement.id as usize,
				timestamp: placement.timestamp as u32,
			})
			.map(|token| {
				let mut uri = format!(
					"/boards/{}/pixels?page={}&limit={}",
					board_id, token, limit,
				);

				if !filter.color.is_open() {
					uri.push_str(&format!("&color={}", filter.color))
				}
				if let Some(user) = filter.user {
					if let Some(user) = byte_serialize(user.as_bytes()).next() {
						uri.push_str(&format!("&user={}", user))
					}
				}
				if !filter.position.is_open() {
					uri.push_str(&format!("&position={}", filter.position))
				}
				if !filter.timestamp.is_open() {
					uri.push_str(&format!("&timestamp={}", filter.timestamp))
				}

				uri.parse().unwrap()
			});

		let mut items = Vec::with_capacity(limit);

		for (placement, user) in placements.into_iter().take(limit) {
			let user = user.unwrap();
			items.push(Placement {
				position: placement.position as u64,
				color: placement.color as u8,
				modified: placement.timestamp as u32,
				user: Reference::from(User::from(user)),
			})
		}

		Ok(Page { items, next, previous: None })
	}

	pub async fn get_placement(
		&self,
		board_id: i32,
		position: u64,
	) -> DbResult<Option<Placement>> {
		let placement = placement::Entity::find()
			.find_also_related(user::Entity)
			.filter(
				placement::Column::Board.eq(board_id)
					.and(placement::Column::Position.eq(position as i64)),
			)
			.order_by(placement::Column::Timestamp, sea_orm::Order::Desc)
			.order_by(placement::Column::Id, sea_orm::Order::Desc)
			.one(&self.connection).await?;
		
		if let Some((placement, user)) = placement {
			let user = user.unwrap();
			Ok(Some(Placement {
				position: placement.position as u64,
				color: placement.color as u8,
				modified: placement.timestamp as u32,
				user: Reference::from(User::from(user)),
			}))
		} else {
			Ok(None)
		}
	}

	pub async fn get_two_placements(
		&self,
		board_id: i32,
		position: u64,
	) -> DbResult<(Option<LastPlacement>, Option<LastPlacement>)> {
		let placements = placement::Entity::find()
			.filter(
				placement::Column::Board.eq(board_id)
					.and(placement::Column::Position.eq(position as i64)),
			)
			.order_by(placement::Column::Timestamp, sea_orm::Order::Desc)
			.order_by(placement::Column::Id, sea_orm::Order::Desc)
			.limit(2)
			.all(&self.connection).await?;

		let mut pair = Vec::with_capacity(2);
		for placement in placements {
			let placement = LastPlacement {
				id: placement.id,
				modified: placement.timestamp as _,
				color: placement.color as _,
				user: UserSpecifier(placement.user_id),
			};
			pair.push(placement)
		}
		let mut pair = pair.into_iter();
		Ok((pair.next(), pair.next()))
	}

	pub async fn delete_placement(&self, placement_id: i64,) -> DbResult<()> {
		placement::Entity::delete_by_id(placement_id)
			.exec(&self.connection).await?;
		Ok(())
	}

	pub async fn insert_placements(
		&self,
		board_id: i32,
		placements: &[PendingPlacement],
	) -> DbResult<()> {
		placement::Entity::insert_many(
			placements.iter().map(|p| {
				placement::ActiveModel {
					id: NotSet,
					board: Set(board_id),
					position: Set(p.position as i64),
					color: Set(p.color as i16),
					timestamp: Set(p.timestamp as i32),
					user_id: Set(p.user.0),
				}
			})
		)
		.exec(&self.connection).await
		.map(|_| ())
		.map_err(DatabaseError::from)
	}

	/// use density buffer instead
	#[deprecated]
	pub async fn count_placements(
		&self,
		board_id: i32,
		position: u64,
		timestamp: u32,
	) -> DbResult<usize> {
		placement::Entity::find()
			.filter(
				placement::Column::Position.eq(position as i64)
				.and(placement::Column::Timestamp.lt(timestamp as i32))
				.and(placement::Column::Board.eq(board_id))
			)
			.count(&self.connection).await
			.map(|i| i as usize)
			.map_err(DatabaseError::from)
	}

	pub async fn list_user_placements(
		&self,
		board_id: i32,
		user: &UserSpecifier,
		limit: usize,
	) -> DbResult<Vec<CachedPlacement>> {
		let placements = placement::Entity::find()
			.filter(
				placement::Column::Board.eq(board_id)
					.and(placement::Column::UserId.eq(user)),
			)
			.left_join(user::Entity)
			.order_by(placement::Column::Timestamp, sea_orm::Order::Desc)
			.order_by(placement::Column::Id, sea_orm::Order::Desc)
			.limit(Some(limit as u64))
			.all(&self.connection).await?;

		Ok(placements.into_iter().rev().map(|placement| CachedPlacement {
			position: placement.position as _,
			modified: placement.timestamp as _,
			user: UserSpecifier(placement.user_id),
		}).collect())
	}

	pub async fn user_count_between(
		&self,
		board_id: i32,
		min_time: i32,
		max_time: i32,
	) -> DbResult<usize> {
		placement::Entity::find()
			.distinct_on([placement::Column::UserId])
			.filter(placement::Column::Board.eq(board_id))
			.filter(placement::Column::Timestamp.between(min_time, max_time))
			.count(&self.connection).await
			.map(|count| count as usize)
			.map_err(DatabaseError::from)
	}

	pub async fn density_for_time(
		&self,
		board_id: i32,
		position: i64,
		max_time: i32,
	) -> DbResult<u32> {
		placement::Entity::find()
			.distinct_on([placement::Column::UserId])
			.filter(placement::Column::Board.eq(board_id))
			.filter(placement::Column::Position.eq(position))
			.filter(placement::Column::Timestamp.lt(max_time))
			.count(&self.connection).await
			.map(|count| u32::try_from(count).expect("Board too dense"))
			.map_err(DatabaseError::from)
	}

	pub async fn create_sector(
		&self,
		board_id: i32,
		index: i32,
		mask: Vec<u8>,
		initial: Vec<u8>,
	) -> DbResult<Sector> {

		let new_sector = board_sector::ActiveModel {
			board: Set(board_id),
			sector: Set(index),
			mask: Set(mask),
			initial: Set(initial),
		};

		let sector = board_sector::Entity::insert(new_sector)
			.exec_with_returning(&self.connection).await?;

		self.sector_from_model(sector).await
	}

	pub async fn get_sector(
		&self,
		board_id: i32,
		sector_index: i32,
	) -> DbResult<Option<Sector>> {
		let sector = board_sector::Entity::find_by_id((board_id, sector_index))
			.one(&self.connection).await?;

		match sector {
			Some(sector) => self.sector_from_model(sector).await.map(Some),
			None => Ok(None),
		}
	}

	async fn sector_from_model(
		&self,
		sector: board_sector::Model,
	) -> DbResult<Sector> {
		let index = sector.sector;
		let board = sector.board;
		let sector_size = sector.initial.len();

		let initial = BytesMut::from(&*sector.initial);
		let mask = BytesMut::from(&*sector.mask);
		let mut colors = initial.clone();
		let mut timestamps = BytesMut::from(&vec![0; sector_size * 4][..]);
		let mut density = BytesMut::from(&vec![0; sector_size * 4][..]);

		let start_position = sector_size as i64 * sector.sector as i64;
		let end_position = start_position + sector_size as i64 - 1;

		let column_timestamp_id_pair = Expr::tuple([
			Expr::col(placement::Column::Timestamp).into(),
			Expr::col(placement::Column::Id).into(),
		]);

		// TODO: look into storing this as indices on the database to skip
		// loading all placements.
		let mut placements = placement::Entity::find()
			.filter(placement::Column::Board.eq(board))
			.filter(placement::Column::Position.between(start_position, end_position))
			.order_by_asc(column_timestamp_id_pair)
			.stream(&self.connection).await?;

		while let Some(placement) = placements.try_next().await? {
			let index = placement.position as usize % sector_size;
			colors[index] = placement.color as u8;
			
			let index4 = index * 4..index * 4 + 4;
			let mut timestamp_slice = &mut timestamps[index4.clone()];
			timestamp_slice.put_u32_le(placement.timestamp as u32);

			let current_density = u32::from_le_bytes(unsafe {
				density[index4.clone()].try_into().unwrap_unchecked()
			});
			let mut density_slice = &mut density[index4];
			density_slice.put_u32_le(current_density + 1);
		}
		
		let initial = WriteBuffer::new(initial);
		let mask = WriteBuffer::new(mask);
		let colors = WriteBuffer::new(colors);
		let timestamps = WriteBuffer::new(timestamps);
		let density = WriteBuffer::new(density);

		Ok(Sector {
			board,
			index,
			initial,
			mask,
			colors,
			timestamps,
			density,
		})
	}

	fn find_sector(board_id: i32, sector_index: i32) -> SimpleExpr {
		board_sector::Column::Sector
			.eq(sector_index)
			.and(board_sector::Column::Board.eq(board_id))
	}

	pub async fn write_sector_mask(
		&self,
		board_id: i32,
		sector_index: i32,
		mask: Vec<u8>,
	) -> DbResult<()> {
		board_sector::Entity::update_many()
			.col_expr(board_sector::Column::Mask, mask.into())
			.filter(Self::find_sector(board_id, sector_index))
			.exec(&self.connection).await
			.map(|_| ())
			.map_err(DatabaseError::from)
	}

	pub async fn write_sector_initial(
		&self,
		board_id: i32,
		sector_index: i32,
		initial: Vec<u8>,
	) -> DbResult<()> {
		board_sector::Entity::update_many()
			.col_expr(board_sector::Column::Initial, initial.into())
			.filter(Self::find_sector(board_id, sector_index))
			.exec(&self.connection).await
			.map(|_| ())
			.map_err(DatabaseError::from)
	}

	pub async fn list_notices(
		&self,
		token: NoticePageToken,
		limit: usize,
		filter: NoticeFilter,
	) -> DbResult<Page<Reference<Notice>>> {
		let column_timestamp_id_pair = Expr::tuple([
			Expr::col(notice::Column::CreatedAt).into(),
			Expr::col(notice::Column::Id).into(),
		]);

		let value_timestamp_id_pair = Expr::tuple([
			(token.timestamp as i64).into(),
			(token.id as i32).into(),
		]);

		let notices = notice::Entity::find()
			.find_also_related(user::Entity)
			.filter(Expr::gte(column_timestamp_id_pair.clone(), value_timestamp_id_pair))
			.apply_if(filter.author.as_ref(), |q, id| q.filter(notice::Column::Author.eq(id)))
			.apply_if(filter.content.as_ref(), |q, content| q.filter(notice::Column::Content.eq(content)))
			.apply_if(filter.title.as_ref(), |q, title| q.filter(notice::Column::Title.eq(title)))
			.apply_if(filter.created_at.start, |q, start| q.filter(notice::Column::CreatedAt.gte(start)))
			.apply_if(filter.created_at.end, |q, end| q.filter(notice::Column::CreatedAt.lte(end)))
			.apply_if(filter.expires_at.start, |q, start| q.filter(notice::Column::ExpiresAt.gte(start)))
			.apply_if(filter.expires_at.end, |q, end| q.filter(notice::Column::ExpiresAt.lte(end)))
			.order_by(column_timestamp_id_pair, sea_orm::Order::Asc)
			.limit(limit as u64 + 1)
			.all(&self.connection).await?;

		let next = notices.windows(2).nth(limit.saturating_sub(1))
			.map(|pair| &pair[0])
			.map(|(notice, _)| NoticePageToken {
				id: notice.id as _,
				timestamp: notice.created_at as _,
			})
			.map(|token| {
				format!( // TODO: filters
					"/notices?page={}&limit={}",
					token, limit,
				).parse().unwrap()
			});

		let notices = notices.into_iter()
			.take(limit)
			.map(|(notice, author)| (notice.id, Notice {
				title: notice.title,
				content: notice.content,
				created_at: notice.created_at as _,
				expires_at: notice.expires_at.map(|v| v as _),
				author: author.map(User::from).map(Reference::from),
			}))
			.map(|(id, notice)| Reference::new(Notice::uri(id), notice))
			.collect();
		
		Ok(Page { items: notices, next, previous: None })
	}

	pub async fn get_notice(
		&self,
		id: usize,
	) -> DbResult<Option<Notice>> {
		notice::Entity::find_by_id(id as i32)
			.find_also_related(user::Entity)
			.one(&self.connection).await
			.map(|n| n.map(|(notice, author)| Notice {
				title: notice.title,
				content: notice.content,
				created_at: notice.created_at as _,
				expires_at: notice.expires_at.map(|v| v as _),
				author: author.map(User::from).map(Reference::from),
			}))
			.map_err(DatabaseError::from)
	}

	pub async fn create_notice(
		&self,
		title: String,
		content: String,
		expiry: Option<u64>,
	) -> DbResult<Reference<Notice>> {
		let now = SystemTime::now()
			.duration_since(UNIX_EPOCH)
			.unwrap()
			.as_secs();
		
		let notice = notice::ActiveModel {
			id: NotSet,
			title: Set(title),
			content: Set(content),
			created_at: Set(now as _),
			expires_at: Set(expiry.map(|v| v as _)),
			author: NotSet, // TODO: set this
		};

		notice::Entity::insert(notice)
			.exec_with_returning(&self.connection).await
			.map(|notice| (notice.id, Notice {
				title: notice.title,
				content: notice.content,
				created_at: notice.created_at as _,
				expires_at: notice.expires_at.map(|v| v as _),
				author: None, // TODO: when this is set, retrieve it
			}))
			.map(|(id, notice)| Reference::new(Notice::uri(id), notice))
			.map_err(DatabaseError::from)
	}

	pub async fn edit_notice(
		&self,
		id: usize,
		title: Option<String>,
		content: Option<String>,
		expiry: Option<Option<u64>>,
	) -> DbResult<Reference<Notice>> {
		let notice = notice::ActiveModel {
			id: Set(id as _),
			title: title.map(Set).unwrap_or(NotSet),
			content: content.map(Set).unwrap_or(NotSet),
			created_at: NotSet,
			expires_at: expiry.map(|e| Set(e.map(|v| v as _))).unwrap_or(NotSet),
			author: NotSet, // TODO: set this
		};
		
		notice::Entity::update(notice)
			.exec(&self.connection).await
			.map(|notice| (notice.id, Notice {
				title: notice.title,
				content: notice.content,
				created_at: notice.created_at as _,
				expires_at: notice.expires_at.map(|v| v as _),
				author: None, // TODO: when this is set, retrieve it
			}))
			.map(|(id, notice)| Reference::new(Notice::uri(id), notice))
			.map_err(DatabaseError::from)
	}

	// returns Ok(true) if the item was deleted or Ok(false) if it didn't exist
	pub async fn delete_notice(
		&self,
		id: usize,
	) -> DbResult<bool> {
		notice::Entity::delete_by_id(id as i32)
			.exec(&self.connection).await
			.map(|result| result.rows_affected == 1)
			.map_err(DatabaseError::from)
	}

	pub async fn list_board_notices(
		&self,
		board_id: i32,
		token: BoardsNoticePageToken,
		limit: usize,
		filter: BoardNoticeFilter,
	) -> DbResult<Page<Reference<BoardsNotice>>> {
		let column_timestamp_id_pair = Expr::tuple([
			Expr::col(board_notice::Column::CreatedAt).into(),
			Expr::col(board_notice::Column::Id).into(),
		]);

		let value_timestamp_id_pair = Expr::tuple([
			(token.timestamp as i64).into(),
			(token.id as i32).into(),
		]);

		let notices = board_notice::Entity::find()
			.find_also_related(user::Entity)
			.filter(board_notice::Column::Board.eq(board_id))
			.filter(Expr::gte(column_timestamp_id_pair.clone(), value_timestamp_id_pair))
			.apply_if(filter.author.as_ref(), |q, id| q.filter(board_notice::Column::Author.eq(id)))
			.apply_if(filter.content.as_ref(), |q, content| q.filter(board_notice::Column::Content.eq(content)))
			.apply_if(filter.title.as_ref(), |q, title| q.filter(board_notice::Column::Title.eq(title)))
			.apply_if(filter.created_at.start, |q, start| q.filter(board_notice::Column::CreatedAt.gte(start)))
			.apply_if(filter.created_at.end, |q, end| q.filter(board_notice::Column::CreatedAt.lte(end)))
			.apply_if(filter.expires_at.start, |q, start| q.filter(board_notice::Column::ExpiresAt.gte(start)))
			.apply_if(filter.expires_at.end, |q, end| q.filter(board_notice::Column::ExpiresAt.lte(end)))
			.order_by(column_timestamp_id_pair, sea_orm::Order::Asc)
			.limit(limit as u64 + 1)
			.all(&self.connection).await?;

		let next = notices.windows(2).nth(limit.saturating_sub(1))
			.map(|pair| &pair[0])
			.map(|(notice, _)| BoardsNoticePageToken {
				id: notice.id as _,
				timestamp: notice.created_at as _,
			})
			.map(|token| {
				format!( // TODO: filter
					"/boards/{}/notices?page={}&limit={}",
					board_id, token, limit,
				).parse().unwrap()
			});
		
		let notices = notices.into_iter()
			.take(limit)
			.map(|(notice, author)| (notice.id, BoardsNotice {
				title: notice.title,
				content: notice.content,
				created_at: notice.created_at as _,
				expires_at: notice.expires_at.map(|v| v as _),
				author: author.map(User::from).map(Reference::from),
			}))
			.map(|(id, notice)| Reference::new(BoardsNotice::uri(board_id, id), notice))
			.collect();
		
		Ok(Page { items: notices, next, previous: None })
	}

	pub async fn get_board_notice(
		&self,
		board_id: i32,
		id: usize,
	) -> DbResult<Option<BoardsNotice>> {
		let notice = board_notice::Entity::find_by_id(id as i32)
			.find_also_related(user::Entity)
			.filter(board_notice::Column::Board.eq(board_id))
			.one(&self.connection).await?
			.map(|(notice, author)| BoardsNotice {
				title: notice.title,
				content: notice.content,
				created_at: notice.created_at as _,
				expires_at: notice.expires_at.map(|v| v as _),
				author: author.map(User::from).map(Reference::from),
			});
		
		Ok(notice)
	}

	pub async fn create_board_notice(
		&self,
		board_id: i32,
		title: String,
		content: String,
		expiry: Option<u64>,
		author: Option<&User>,
	) -> DbResult<Reference<BoardsNotice>> {
		let now = SystemTime::now()
			.duration_since(UNIX_EPOCH)
			.unwrap()
			.as_secs();
		
		let notice = board_notice::ActiveModel {
			id: NotSet,
			board: Set(board_id),
			title: Set(title),
			content: Set(content),
			created_at: Set(now as _),
			expires_at: Set(expiry.map(|v| v as _)),
			author: Set(author.map(|u| u.id)),
		};

		board_notice::Entity::insert(notice)
			.exec_with_returning(&self.connection).await
			.map(|notice| (notice.id, BoardsNotice {
				title: notice.title,
				content: notice.content,
				created_at: notice.created_at as _,
				expires_at: notice.expires_at.map(|v| v as _),
				author: author.map(|a| a.clone().into()),
			}))
			.map(|(id, notice)| Reference::new(BoardsNotice::uri(board_id, id), notice))
			.map_err(DatabaseError::from)
	}

	pub async fn edit_board_notice(
		&self,
		board_id: i32,
		id: usize,
		title: Option<String>,
		content: Option<String>,
		expiry: Option<Option<u64>>,
	) -> DbResult<Reference<BoardsNotice>> {
		let notice = board_notice::ActiveModel {
			board: NotSet,
			id: Set(id as _),
			title: title.map(Set).unwrap_or(NotSet),
			content: content.map(Set).unwrap_or(NotSet),
			created_at: NotSet,
			expires_at: expiry.map(|e| Set(e.map(|v| v as _))).unwrap_or(NotSet),
			author: NotSet, // TODO: set this
		};
		
		board_notice::Entity::update(notice)
			.filter(board_notice::Column::Board.eq(board_id))
			.exec(&self.connection).await
			.map(|notice| (notice.id, BoardsNotice {
				title: notice.title,
				content: notice.content,
				created_at: notice.created_at as _,
				expires_at: notice.expires_at.map(|v| v as _),
				author: None, // TODO: when this is set, it will have to be fetched
			}))
			.map(|(id, notice)| Reference::new(BoardsNotice::uri(board_id, id), notice))
			.map_err(DatabaseError::from)
	}

	// returns Ok(true) if the item was deleted or Ok(false) if it didn't exist
	pub async fn delete_board_notice(
		&self,
		board_id: i32,
		id: usize,
	) -> DbResult<bool> {
		board_notice::Entity::delete_by_id(id as i32)
			.filter(board_notice::Column::Board.eq(board_id))
			.exec(&self.connection).await
			.map(|result| result.rows_affected == 1)
			.map_err(DatabaseError::from)
	}

	pub async fn list_reports(
		&self,
		token: ReportPageToken,
		limit: usize,
		filter: ReportFilter,
		owner: Option<Option<&User>>,
	) -> DbResult<Page<Reference<Report>>> {
		let transaction = self.connection.begin().await?;

		let list = report::Entity::find()
			.find_also_related(user::Entity)
			.distinct_on([report::Column::Id])
			.filter(report::Column::Id.gt(token.0 as i64))
			.apply_if(filter.status.as_ref(), |q, status| q.filter(report::Column::Closed.eq(matches!(status, ReportStatus::Closed))))
			.apply_if(filter.reason.as_ref(), |q, reason| q.filter(report::Column::Reason.eq(reason)))
			.apply_if(owner, |q, owner| q.filter(report::Column::Reporter.eq(owner.map(|o| o.id))))
			.order_by(report::Column::Id, sea_orm::Order::Asc)
			.order_by(report::Column::Revision, sea_orm::Order::Desc)
			.limit(limit as u64 + 1)
			.all(&transaction).await?;

		let next = list.windows(2).nth(limit.saturating_sub(1))
			.map(|pair| &pair[0])
			.map(|(report, _)| ReportPageToken(report.id as _))
			.map(|token| {
				format!( // TODO: filter
					"/reports?page={}&limit={}",
					token, limit,
				).parse().unwrap()
			});

		let mut reports = vec![];

		for (report, reporter) in list.into_iter().take(limit) {
			let artifacts = report.find_related(report_artifact::Entity)
				.all(&transaction).await?
				.into_iter()
				.map(|a| Artifact::parse(&a.uri, a.timestamp as _))
				.collect::<Result<_, _>>()
				.map_err(|_| sea_orm::DbErr::Custom("integrity error".to_string()))?;

			let id = report.id;
			let report = Report {
				status: if report.closed { ReportStatus::Closed } else { ReportStatus::Opened },
				reason: report.reason,
				reporter: reporter.map(User::from).map(Reference::from),
				artifacts,
				timestamp: report.timestamp as _,
			};
			reports.push(Reference::new(Report::uri(id), report))
		}

		transaction.commit().await?;
		
		Ok(Page { items: reports, next, previous: None })
	}

	pub async fn get_report(
		&self,
		id: usize,
	) -> DbResult<Option<Report>> {
		let transaction = self.connection.begin().await?;

		let report = report::Entity::find()
			.find_also_related(user::Entity)
			.filter(report::Column::Id.eq(id as i32))
			.order_by(report::Column::Revision, sea_orm::Order::Desc)
			.limit(1)
			.one(&transaction).await
			.map_err(DatabaseError::from)?;
		
		match report {
			Some((report, reporter)) => {
				let artifacts = report.find_related(report_artifact::Entity)
					.all(&transaction).await?
					.into_iter()
					.map(|a| Artifact::parse(&a.uri, a.timestamp as _))
					.collect::<Result<_, _>>()
					.map_err(|_| sea_orm::DbErr::Custom("integrity error".to_string()))?;

				transaction.commit().await?;

				let report = Report {
					status: if report.closed { ReportStatus::Closed } else { ReportStatus::Opened },
					reason: report.reason,
					reporter: reporter.map(User::from).map(Reference::from),
					artifacts,
					timestamp: report.timestamp as _,
				};
				Ok(Some(report))
			},
			None => Ok(None),
		}
	}

	pub async fn create_report(
		&self,
		reason: String,
		reporter: Option<&User>,
		artifacts: Vec<Artifact>,
	) -> DbResult<Reference<Report>> {
		let now = SystemTime::now()
			.duration_since(UNIX_EPOCH)
			.unwrap()
			.as_secs();

		let transaction = self.connection.begin().await
			.map_err(DatabaseError::DbErr)?;

		let report = report::Entity::insert(report::ActiveModel {
			id: NotSet,
			revision: Set(1),
			closed: Set(false),
			reason: Set(reason),
			reporter: Set(reporter.map(|r| r.id)),
			timestamp: Set(now as _),
		})
		.exec_with_returning(&transaction).await
		.map_err(DatabaseError::from)?;

		report_artifact::Entity::insert_many(artifacts.iter().map(|a| {
			report_artifact::ActiveModel {
				report: Set(report.id),
				revision: Set(report.revision),
				timestamp: Set(a.timestamp as _),
				uri: Set(a.reference.uri.to_string()),
			}
		})).exec(&transaction).await?;

		let id = report.id;
		let report = Report {
			status: if report.closed { ReportStatus::Closed } else { ReportStatus::Opened },
			reason: report.reason,
			reporter: reporter.map(|r| r.clone().into()),
			artifacts: artifacts.to_owned(),
			timestamp: report.timestamp as _,
		};
		transaction.commit().await?;
		Ok(Reference::new(Report::uri(id), report))
	}

	pub async fn edit_report(
		&self,
		id: usize,
		status: Option<ReportStatus>,
		reason: Option<String>,
		artifacts: Option<Vec<Artifact>>,
	) -> DbResult<Reference<Report>> {
		let now = SystemTime::now()
			.duration_since(UNIX_EPOCH)
			.unwrap()
			.as_secs();

		let transaction = self.begin().await?;

		let old_report = report::Entity::find()
			.filter(report::Column::Id.eq(id as i32))
			.order_by(report::Column::Revision, sea_orm::Order::Desc)
			.one(&transaction.connection).await
			.map_err(DatabaseError::from)?
			.ok_or(DatabaseError::DbErr(DbErr::RecordNotFound("".to_string())))?;

		let artifacts = if let Some(a) = artifacts {
			a
		} else {
			report_artifact::Entity::find()
				.filter(report_artifact::Column::Report.eq(old_report.id))
				.filter(report_artifact::Column::Revision.eq(old_report.revision))
				.all(&transaction.connection).await?
				.into_iter()
				.map(|a| Artifact::parse(&a.uri, a.timestamp as _))
				.collect::<Result<_, _>>()
				.map_err(|_| sea_orm::DbErr::Custom("integrity error".to_string()))?
		};

		let closed = status.map(|s| matches!(s, ReportStatus::Closed))
			.unwrap_or(old_report.closed);
		let report = report::Entity::insert(report::ActiveModel {
			id: Set(id as _),
			revision: Set(old_report.revision + 1),
			closed: Set(closed),
			reason: Set(reason.unwrap_or(old_report.reason)),
			reporter: Set(old_report.reporter),
			timestamp: Set(now as _),
		})
		.exec_with_returning(&transaction.connection).await
		.map_err(DatabaseError::from)?;

		report_artifact::Entity::insert_many(artifacts.iter().map(|a| {
			report_artifact::ActiveModel {
				report: Set(report.id),
				revision: Set(report.revision),
				timestamp: Set(a.timestamp as _),
				uri: Set(a.reference.uri.to_string()),
			}
		})).exec(&transaction.connection).await?;

		let id = report.id;
		
		let reporter = if let Some(reporter) = report.reporter {
			let user = transaction.get_user(&UserSpecifier(reporter)).await?
				.expect("failed to lookup reporting user");
			Some(Reference::from(user))
		} else {
			None
		};
		

		transaction.commit().await?;

		let report = Report {
			status: if report.closed { ReportStatus::Closed } else { ReportStatus::Opened },
			reason: report.reason,
			reporter,
			artifacts,
			timestamp: report.timestamp as _,
		};
		Ok(Reference::new(Report::uri(id), report))
	}

	// returns Some(reporter) if the report was deleted or None if it didn't exist
	pub async fn delete_report(
		&self,
		id: usize,
	) -> DbResult<Option<Option<UserSpecifier>>> {
		let transaction = self.connection.begin().await?;

		let reporter_id = report::Entity::find()
			.find_also_related(user::Entity)
			.filter(report::Column::Id.eq(id as i32))
			.order_by(report::Column::Revision, sea_orm::Order::Desc)
			.limit(1)
			.one(&transaction).await?
			.map(|(_, user)| user.map(|u| UserSpecifier(u.id)));

		let deleted = report::Entity::delete_many()
			.filter(report::Column::Id.eq(id as i32))
			.exec(&transaction).await
			.map(|result| result.rows_affected > 0)?;

		transaction.commit().await?;

		if deleted {
			Ok(Some(reporter_id.unwrap()))
		} else {
			Ok(None)
		}
	}

	pub async fn list_report_history(
		&self,
		id: usize,
		token: ReportPageToken,
		limit: usize,
		filter: ReportFilter,
	) -> DbResult<Page<Report>> {
		let transaction = self.connection.begin().await?;

		let list = report::Entity::find()
			.find_also_related(user::Entity)
			.filter(report::Column::Id.eq(id as i32))
			.filter(report::Column::Revision.gt(token.0 as i64))
			.apply_if(filter.status.as_ref(), |q, status| q.filter(report::Column::Closed.eq(matches!(status, ReportStatus::Closed))))
			.apply_if(filter.reason.as_ref(), |q, reason| q.filter(report::Column::Reason.eq(reason)))
			.order_by(report::Column::Revision, sea_orm::Order::Asc)
			.limit(limit as u64 + 1)
			.all(&transaction).await?;

		let next = list.windows(2).nth(limit.saturating_sub(1))
			.map(|pair| &pair[0])
			.map(|(report, _)| ReportPageToken(report.revision as _))
			.map(|token| {
				format!( // TODO: filter
					"/reports/{}/history?page={}&limit={}",
					id, token, limit,
				).parse().unwrap()
			});

		let mut reports = vec![];

		for (report, reporter) in list.into_iter().take(limit) {
			let artifacts = report.find_related(report_artifact::Entity)
				.all(&transaction).await?
				.into_iter()
				.map(|a| Artifact::parse(&a.uri, a.timestamp as _))
				.collect::<Result<_, _>>()
				.map_err(|_| sea_orm::DbErr::Custom("integrity error".to_string()))?;

			let report = Report {
				status: if report.closed { ReportStatus::Closed } else { ReportStatus::Opened },
				reason: report.reason,
				reporter: reporter.map(User::from).map(Reference::from),
				artifacts,
				timestamp: report.timestamp as _,
			};
			reports.push(report)
		}

		transaction.commit().await?;
		
		Ok(Page { items: reports, next, previous: None })
	}

	pub async fn is_user_banned(&self, user: &UserSpecifier) -> DbResult<bool> {
		let status = BANS_CACHE.check(user, &self.connection).await?;
		match status {
			BanStatus::NotBanned => Ok(false),
			BanStatus::Permabanned => Ok(true),
			BanStatus::BannedUntil(time) => {
				// TODO: make now some shared function somewhere
				let now = SystemTime::now()
					.duration_since(UNIX_EPOCH)
					.unwrap()
					.as_secs();

				Ok(now <= time)
			},
		}
	}

	pub async fn list_user_bans(
		&self,
		user: &UserSpecifier,
		token: BanPageToken,
		limit: usize,
		filter: BanFilter,
	) -> DbResult<Page<Reference<Ban>>> {	
		
		let bans = ban::Entity::find()
			.column_as(Expr::col((BanUser::Table, BanUser::Subject)), "user_subject")
			.column_as(Expr::col((BanUser::Table, BanUser::Name)), "user_name")
			.column_as(Expr::col((BanUser::Table, BanUser::CreatedAt)), "user_created_at")
			.column_as(Expr::col((BanUser::Table, BanIssuer::Subject)), "issuer_subject")
			.column_as(Expr::col((BanUser::Table, BanIssuer::Name)), "issuer_name")
			.column_as(Expr::col((BanUser::Table, BanIssuer::CreatedAt)), "issuer_created_at")
			.join_as(sea_orm::JoinType::InnerJoin, ban::Relation::User.def(), BanUser::Table)
			.join_as(sea_orm::JoinType::LeftJoin, ban::Relation::Issuer.def(), BanIssuer::Table)
			.filter(ban::Column::Id.gt(token.0))
			.filter(ban::Column::UserId.eq(user))
			.apply_if(filter.created_at.start, |q, start| {
				if filter.created_at.end.is_none() {
					q.filter(ban::Column::CreatedAt.gte(start).or(ban::Column::CreatedAt.is_null()))
				} else {
					q.filter(ban::Column::CreatedAt.gte(start))
				}
			})
			.apply_if(filter.created_at.end, |q, end| q.filter(ban::Column::CreatedAt.lte(end)))
			.apply_if(filter.expires_at.start, |q, start| {
				if filter.expires_at.end.is_none() {
					q.filter(ban::Column::ExpiresAt.gte(start).or(ban::Column::ExpiresAt.is_null()))
				} else {
					q.filter(ban::Column::ExpiresAt.gte(start))
				}
			})
			.apply_if(filter.expires_at.end, |q, end| q.filter(ban::Column::ExpiresAt.lte(end)))
			.order_by_asc(ban::Column::Id)
			.limit(Some(limit as u64 + 1))
			.into_model::<BanFull>()
			.all(&self.connection).await?;

		let next = bans.windows(2).nth(limit.saturating_sub(1))
			.map(|pair| &pair[0])
			.map(|ban| BanPageToken(ban.id as _))
			.map(|token| {
				// TODO: filter
				format!("/users/{user}/bans?page={token}&limit={limit}").parse().unwrap()
			});
			
		let mut items = Vec::with_capacity(bans.len());
		for ban in bans.into_iter().take(limit) {
			let (ban, user, issuer) = ban.split();
			let ban = Ban {
				id: ban.id,
				user: User::from(user),
				created_at: ban.created_at as _,
				expires_at: ban.expires_at.map(|e| e as _),
				issuer: issuer.map(User::from).map(Reference::from),
				reason: ban.reason,
			};

			let reference = Reference::from(ban);

			items.push(reference);
		}

		Ok(Page { items, next, previous: None })
	}

	pub async fn get_ban(
		&self,
		ban: &BanSpecifier,
		user: &UserSpecifier,
	) -> DbResult<Option<Ban>> {
		let ban = ban::Entity::find()
			.column_as(Expr::col((BanUser::Table, BanUser::Subject)), "user_subject")
			.column_as(Expr::col((BanUser::Table, BanUser::Name)), "user_name")
			.column_as(Expr::col((BanUser::Table, BanUser::CreatedAt)), "user_created_at")
			.column_as(Expr::col((BanUser::Table, BanIssuer::Subject)), "issuer_subject")
			.column_as(Expr::col((BanUser::Table, BanIssuer::Name)), "issuer_name")
			.column_as(Expr::col((BanUser::Table, BanIssuer::CreatedAt)), "issuer_created_at")
			.join_as(sea_orm::JoinType::InnerJoin, ban::Relation::User.def(), BanUser::Table)
			.join_as(sea_orm::JoinType::LeftJoin, ban::Relation::Issuer.def(), BanIssuer::Table)
			.filter(ban::Column::Id.eq(ban))
			.filter(ban::Column::UserId.eq(user))
			.order_by_asc(ban::Column::Id)
			.limit(1)
			.into_model::<BanFull>()
			.one(&self.connection).await
			.map_err(DatabaseError::from)?
			.map(|b| b.split())
			.map(|(ban, user, issuer)| Ban {
				id: ban.id,
				user: User::from(user),
				created_at: ban.created_at as _,
				expires_at: ban.expires_at.map(|e| e as _),
				issuer: issuer.map(User::from).map(Reference::from),
				reason: ban.reason,
			});
		
		Ok(ban)
	}

	pub async fn create_ban(
		&self,
		user: &UserSpecifier,
		issuer: Option<&UserSpecifier>,
		reason: Option<String>,
		expiry: Option<u64>,
	) -> DbResult<TryInsertResult<Ban>> {
		let now = SystemTime::now()
			.duration_since(UNIX_EPOCH)
			.unwrap()
			.as_secs();
		
		let ban = ban::ActiveModel {
			id: NotSet,
			user_id: Set(user.0),
			created_at: Set(now as _),
			expires_at: Set(expiry.map(|e| e as _)),
			issuer: Set(issuer.map(|i| i.0)),
			reason: Set(reason),
		};
		
		let transaction = self.begin().await?;
		
		let user = match transaction.get_user(user).await? {
			Some(user) => user,
			None => return Ok(TryInsertResult::Empty),
		};
		
		let issuer = if let Some(issuer) = issuer {
			match transaction.get_user(issuer).await? {
				Some(user) => Some(Reference::from(user)),
				None => return Ok(TryInsertResult::Empty),
			}
		} else {
			None
		};
		
		let insert = ban::Entity::insert(ban)
			.exec_with_returning(&transaction.connection).await?;
		
		transaction.commit().await?;
		BANS_CACHE.invalidate(&user.specifier()).await;

		let ban = Ban {
			id: insert.id,
			user,
			created_at: insert.created_at as _,
			expires_at: insert.expires_at.map(|i| i as _),
			issuer,
			reason: insert.reason,
		};
		Ok(TryInsertResult::Inserted(ban))
	}

	pub async fn edit_ban(
		&self,
		ban: &BanSpecifier,
		user: &UserSpecifier,
		reason: Option<Option<String>>,
		expiry: Option<Option<u64>>,
	) -> DbResult<Option<Ban>> {
		let model = ban::ActiveModel {
			id: Set(ban.0),
			user_id: NotSet,
			created_at: NotSet,
			expires_at: expiry.map(|ex| Set(ex.map(|e| e as _))).unwrap_or(NotSet),
			issuer: NotSet,
			reason: reason.map(Set).unwrap_or(NotSet),
		};
		
		let transaction = self.begin().await?;
		
		let user = match transaction.get_user(user).await? {
			Some(user) => user,
			None => return Ok(None),
		};
		
		let update = ban::Entity::update(model)
			.filter(ban::Column::UserId.eq(user.id))
			.filter(ban::Column::Id.eq(ban.0))
			.exec(&transaction.connection).await?;
		
		let issuer = if let Some(issuer) = update.issuer {
			let user = transaction.get_user(&UserSpecifier(issuer)).await?
				.expect("failed to lookup ban issuer");
			Some(Reference::from(user))
		} else {
			None
		};
		
		transaction.commit().await?;
		
		BANS_CACHE.invalidate(&user.specifier()).await;

		let ban = Ban {
			id: ban.0,
			user,
			created_at: update.created_at as _,
			expires_at: update.expires_at.map(|i| i as _),
			issuer,
			reason: update.reason,
		};

		Ok(Some(ban))
	}

	pub async fn delete_ban(
		&self,
		ban: &BanSpecifier,
		user: &UserSpecifier,
	) -> DbResult<bool> {
		let delete = ban::Entity::delete_by_id(ban.0)
			.filter(ban::Column::UserId.eq(user))
			.exec(&self.connection).await;

		BANS_CACHE.invalidate(user).await;

		delete.map(|result| result.rows_affected == 1)
			.map_err(DatabaseError::from)
	}

	
	pub async fn list_users(
		&self,
		page: UsersPageToken,
		limit: usize,
		filter: UserFilter,
	) -> DbResult<Page<Reference<User>>> {
		let users = user::Entity::find()
			.filter(user::Column::Id.gt(page.0))
			.apply_if(filter.name, |q, name| q.filter(user::Column::Name.like(format!("%{name}%"))))
			.apply_if(filter.created_at.start, |q, start| {
				if filter.created_at.end.is_none() {
					q.filter(user::Column::CreatedAt.gte(start).or(user::Column::CreatedAt.is_null()))
				} else {
					q.filter(user::Column::CreatedAt.gte(start))
				}
			})
			.apply_if(filter.created_at.end, |q, end| q.filter(user::Column::CreatedAt.lte(end)))
			.order_by(user::Column::Id, sea_orm::Order::Asc)
			.limit(Some((limit + 1) as u64))
			.all(&self.connection).await?;
		
		let next = users.windows(2).nth(limit.saturating_sub(1))
			.map(|pair| &pair[0])
			.map(UsersPageToken::from)
			.map(|token| {
				 // TODO: filter
				format!("/users?page={token}&limit={limit}").parse().unwrap()
			});
		
		let items = users.into_iter()
			.take(limit)
			.map(User::from)
			.map(Reference::from)
			.collect();
		
		// TODO: previous
		Ok(Page { items, next, previous: None })
	}
	
	pub async fn create_user(
		&self,
		subject: String,
		username: String,
		created_at: SystemTime,
	) -> DbResult<User> {
		let created_at = created_at
			.duration_since(UNIX_EPOCH)
			.unwrap()
			.as_secs();
		
		let model = user::ActiveModel {
			subject: Set(subject.clone()),
			name: Set(username),
			created_at: Set(created_at as i64),
			..Default::default()
		};
		
		let transaction = self.begin().await?;
		
		let user = user::Entity::find()
			.filter(user::Column::Subject.eq(subject))
			.one(&transaction.connection).await?;
		
		if let Some(existing) = user {
			transaction.commit().await?;
			Ok(User::from(existing))
		} else {
			let insert = user::Entity::insert(model)
				.exec_with_returning(&transaction.connection).await?;
			
			transaction.commit().await?;
			Ok(User::from(insert))
		}
	}
	
	// TODO: cache
	pub async fn get_user(
		&self,
		user: &UserSpecifier,
	) -> DbResult<Option<User>> {
		let user = user::Entity::find()
			.filter(user::Column::Id.eq(user))
			.one(&self.connection).await?;
		
		Ok(user.map(User::from))
	}
	
	pub async fn update_user(
		&self,
		user: &UserSpecifier,
		name: &str,
	) -> DbResult<Option<User>> {
		let model = user::ActiveModel {
			name: Set(name.to_owned()),
			..Default::default()
		};
		
		let update = user::Entity::update_many()
			.set(model)
			.filter(user::Column::Id.eq(user))
			.exec_with_returning(&self.connection).await?;
		
		match update.as_slice() {
			[] => Ok(None),
			[user] => Ok(Some(User::from(user.clone()))),
			_ => panic!("updated multiple users with the same subject"),
		}
	}
	
	pub async fn delete_user(
		&self,
		user: &UserSpecifier,
	) -> DbResult<Option<()>> {
		let model = user::ActiveModel {
			id: Set(user.0),
			..Default::default()
		};
		
		let delete = user::Entity::delete(model)
			.exec(&self.connection).await?;
		
		match delete.rows_affected {
			0 => Ok(None),
			1 => Ok(Some(())),
			_ => panic!("deleted multiple users with the same subject"),
		}
	}
	
	pub async fn list_roles(
		&self,
		page: RolesPageToken,
		limit: usize,
		filter: RoleFilter,
	) -> DbResult<Page<Reference<Role>>> {
		let roles = role::Entity::find()
			.filter(role::Column::Id.gt(page.0))
			.apply_if(filter.name, |q, name| q.filter(role::Column::Name.like(format!("%{name}%"))))
			.apply_if(filter.icon, |q, icon| q.filter(role::Column::Icon.like(format!("%{icon}%"))))
			.order_by(role::Column::Id, sea_orm::Order::Asc)
			.limit(Some((limit + 1) as u64))
			.all(&self.connection).await?;
		
		let next = roles.windows(2).nth(limit.saturating_sub(1))
			.map(|pair| &pair[0])
			.map(RolesPageToken::from)
			.map(|token| {
				// TODO: filter
				format!("/roles?page={token}&limit={limit}").parse().unwrap()
			});
		
		let items = roles.into_iter()
			.take(limit)
			.map(Role::from)
			.map(Reference::from)
			.collect();
		
		// TODO: previous
		Ok(Page { items, next, previous: None })
	}
	
	pub async fn get_role(
		&self,
		role: &RoleSpecifier,
	) -> DbResult<Option<Role>> {
		role::Entity::find()
			.filter(role::Column::Id.eq(role.0))
			.one(&self.connection).await
			.map(|r| r.map(Role::from))
			.map_err(DatabaseError::from)
	}
	
	pub async fn create_role(
		&self,
		name: String,
		icon: Option<Url>,
		permissions: EnumSet<Permission>,
	) -> DbResult<Reference<Role>> {
		let icon = icon.map(String::from);
		let permissions = permissions.iter()
			.map(|p| <&str>::from(&p))
			.map(String::from)
			.collect::<Vec<_>>()
			.join(",");
		
		let role = role::ActiveModel { 
			name: Set(name), 
			icon: Set(icon), 
			permissions: Set(permissions),
			..Default::default()
		};
		
		let insert = role::Entity::insert(role)
			.exec_with_returning(&self.connection).await?;
		
		Ok(Reference::from(Role::from(insert)))
	}
	
	pub async fn update_role(
		&self,
		role: &RoleSpecifier,
		name: Option<String>,
		icon: Option<Option<Url>>,
		permissions: Option<EnumSet<Permission>>,
	) -> DbResult<Option<Reference<Role>>> {
		let model = role::ActiveModel { 
			name: name.map(Set).unwrap_or(NotSet),
			icon: icon.map(|icon| Set(icon.map(String::from))).unwrap_or(NotSet),
			permissions: permissions.map(|p| {
				p.iter()
					.map(|p| <&str>::from(&p))
					.collect::<Vec<_>>()
					.join(",")
			}).map(Set).unwrap_or(NotSet),
			..Default::default()
		};
		
		let update = role::Entity::update_many()
			.set(model)
			.filter(role::Column::Id.eq(role.0))
			.exec_with_returning(&self.connection).await?;
		
		match update.as_slice() {
			[] => Ok(None),
			[user] => Ok(Some(Reference::from(Role::from(user.clone())))),
			_ => panic!("updated multiple roles with the same name"),
		}
	}
	
	pub async fn delete_role(
		&self,
		role: &RoleSpecifier,
	) -> DbResult<Option<()>> {
		let delete = role::Entity::delete_by_id(role.0)
			.exec(&self.connection).await?;
		
		match delete.rows_affected {
			0 => Ok(None),
			1 => Ok(Some(())),
			_ => panic!("deleted multiple roles with the same name"),
		}
	}
	
	
	pub async fn list_factions(
		&self,
		page: FactionsPageToken,
		limit: usize,
		filter: FactionFilter,
	) -> DbResult<Page<Reference<Faction>>> {
		let factions = faction::Entity::find()
			.column_as(faction_member::Column::Member.count(), "size")
			.join(sea_orm::JoinType::FullOuterJoin, faction::Relation::FactionMember.def())
			.group_by(faction::Column::Id)
			.filter(faction::Column::Id.gt(page.0))
			.apply_if(filter.name, |q, name| {
				let query_builder = self.connection
					.get_database_backend()
					.get_query_builder();
				let filter = name.split('*')
					.map(|part| query_builder.escape_string(part))
					.collect::<Vec<_>>()
					.join("%");
				// TODO: apply same filtering logic to other searches and also do case insenstivity
				q.filter(faction::Column::Name.like(filter))
			})
			.apply_if(filter.created_at.start, |q, start| {
				if filter.created_at.end.is_none() {
					q.filter(faction::Column::CreatedAt.gte(start).or(faction::Column::CreatedAt.is_null()))
				} else {
					q.filter(faction::Column::CreatedAt.gte(start))
				}
			})
			.apply_if(filter.created_at.end, |q, end| q.filter(faction::Column::CreatedAt.lte(end)))
			.apply_if(filter.size.start, |q, start| {
				let expr = SimpleExpr::Binary(
					Box::new(SimpleExpr::Column(ColumnRef::Column("size".into_iden()))),
					sea_query::BinOper::GreaterThanOrEqual,
					Box::new(SimpleExpr::Constant((start as i64).into()))
				);
				q.filter(expr)
			})
			.apply_if(filter.size.end, |q, end| {
				let expr = SimpleExpr::Binary(
					Box::new(SimpleExpr::Column(ColumnRef::Column("size".into_iden()))),
					sea_query::BinOper::SmallerThanOrEqual,
					Box::new(SimpleExpr::Constant((end as i64).into()))
				);
				q.filter(expr)
			})
			.order_by(faction::Column::Id, sea_orm::Order::Asc)
			.limit(Some((limit + 1) as u64))
			.into_model::<FactionFull>()
			.all(&self.connection).await?;
		
		let next = factions.windows(2).nth(limit.saturating_sub(1))
			.map(|pair| &pair[0])
			.map(FactionsPageToken::from)
			.map(|token| {
				// TODO: filter
				format!("/factions?page={token}&limit={limit}").parse().unwrap()
			});
		
		let items = factions.into_iter()
			.take(limit)
			.map(Faction::from)
			.map(Reference::from)
			.collect();
		
		// TODO: previous
		Ok(Page { items, next, previous: None })
	}
	
	pub async fn get_faction(
		&self,
		faction: &FactionSpecifier,
	) -> DbResult<Option<Faction>> {
		faction::Entity::find()
			.column_as(faction_member::Column::Member.count(), "size")
			.join(sea_orm::JoinType::FullOuterJoin, faction::Relation::FactionMember.def())
			.group_by(faction::Column::Id)
			.filter(faction::Column::Id.eq(faction))
			.into_model::<FactionFull>()
			.one(&self.connection).await
			.map(|r| r.map(Faction::from))
			.map_err(DatabaseError::from)
	}
	
	pub async fn create_faction(
		&self,
		name: String,
		icon: Option<Url>,
	) -> DbResult<Reference<Faction>> {
		let icon = icon.map(String::from);
		
		let now = SystemTime::now()
			.duration_since(UNIX_EPOCH)
			.unwrap()
			.as_secs();
		
		let faction = faction::ActiveModel { 
			name: Set(name),
			icon: Set(icon),
			created_at: Set(now as _),
			..Default::default()
		};
		
		let insert = faction::Entity::insert(faction)
			.exec_with_returning(&self.connection).await?;
		
		let faction_full = FactionFull::from_model_and_size(insert, 0);
		
		Ok(Reference::from(Faction::from(faction_full)))
	}
	
	pub async fn update_faction(
		&self,
		faction: &FactionSpecifier,
		name: Option<String>,
		icon: Option<Option<Url>>,
	) -> DbResult<Option<Faction>> {
		let model = faction::ActiveModel { 
			name: name.map(Set).unwrap_or(NotSet),
			icon: icon.map(|icon| Set(icon.map(String::from))).unwrap_or(NotSet),
			..Default::default()
		};
		
		let transaction = self.connection.begin().await?;
		// TODO: this doesn't seem right, but just update has no fail state for not found it seems
		let update = faction::Entity::update_many()
			.set(model)
			.filter(faction::Column::Id.eq(faction))
			.exec_with_returning(&transaction).await?;
		
		match update.as_slice() {
			[] => Ok(None),
			[_] => {
				let faction = faction::Entity::find()
					.column_as(faction_member::Column::Member.count(), "size")
					.join(sea_orm::JoinType::FullOuterJoin, faction::Relation::FactionMember.def())
					.group_by(faction_member::Column::Member)
					.filter(faction::Column::Id.eq(faction))
					.into_model::<FactionFull>()
					.one(&transaction).await?
					.expect("updated a faction which disappeared in a transaction");
				
				transaction.commit().await?;
				Ok(Some(Faction::from(faction)))
			},
			_ => panic!("updated multiple factions with the same id"),
		}
	}
	
	pub async fn delete_faction(
		&self,
		faction: &FactionSpecifier,
	) -> DbResult<Option<()>> {
		let transaction = self.begin().await?;
		
		faction_member::Entity::delete_many()
			.filter(faction_member::Column::Faction.eq(faction.0))
			.exec(&transaction.connection).await?;
		
		let delete = faction::Entity::delete_by_id(faction.0)
			.exec(&transaction.connection).await?;
		
		match delete.rows_affected {
			0 => Ok(None),
			1 => {
				transaction.commit().await?;
				Ok(Some(()))
			},
			_ => panic!("deleted multiple factions with the same id"),
		}
	}
	
	pub async fn anonymous_role(
		&self,
	) -> DbResult<Option<Role>> {
		let anonymous_role = CONFIG.unauthenticated_role.as_ref()
			.or_else(|| CONFIG.default_role.as_ref());
		
		let role_name = match anonymous_role {
			Some(role) => role,
			None => return Ok(None),
		};
		
		let role = role::Entity::find()
			.filter(role::Column::Name.eq(role_name))
			.one(&self.connection).await?;
		
		Ok(role.map(Role::from))
	}
	
	pub async fn anonymous_permissions(&self) -> DbResult<EnumSet<Permission>> {
		let role = self.anonymous_role().await?;
		Ok(role.map(|r| r.permissions.into_iter().collect()).unwrap_or_default())
	}

	pub async fn user_permissions(
		&self,
		user: &UserSpecifier,
	) -> DbResult<EnumSet<Permission>> {
		let default_role = CONFIG.default_role.as_ref();
		
		let roles = role::Entity::find()
			.join(sea_orm::JoinType::FullOuterJoin, role::Relation::RoleMember.def())
			.apply_if(default_role.is_none().then_some(()), |q, ()| {
				q.filter(role_member::Column::Member.eq(user))
			})
			.apply_if(default_role, |q, role| {
				q.filter(role_member::Column::Member.eq(user).or(role::Column::Name.eq(role)))
			})
			.all(&self.connection).await?;
		
		let permissions = roles.into_iter()
			.map(Role::from)
			.flat_map(|r| r.permissions.into_iter())
			.collect();
		
		Ok(permissions)
	}
	
	pub async fn list_user_roles(
		&self,
		user: &UserSpecifier, 
		page: UserRolesPageToken,
		limit: usize,
		filter: RoleFilter,
	) -> DbResult<Page<Reference<Role>>> {
		let default_role = CONFIG.default_role.as_ref();
		
		let roles = role::Entity::find()
			.join(sea_orm::JoinType::FullOuterJoin, role::Relation::RoleMember.def())
			.apply_if(default_role.is_none().then_some(()), |q, ()| {
				q.filter(role_member::Column::Member.eq(user))
			})
			.apply_if(default_role, |q, role| {
				q.filter(role_member::Column::Member.eq(user).or(role::Column::Name.eq(role)))
			})
			.filter(role::Column::Id.gt(page.0))
			.apply_if(filter.name, |q, name| q.filter(role::Column::Name.like(format!("%{name}%"))))
			.apply_if(filter.icon, |q, icon| q.filter(role::Column::Icon.like(format!("%{icon}%"))))
			.order_by(role::Column::Id, sea_orm::Order::Asc)
			.limit(Some((limit + 1) as _))
			.all(&self.connection).await?;
		
		let next = roles.windows(2).nth(limit.saturating_sub(1))
			.map(|pair| &pair[0])
			.map(UserRolesPageToken::from)
			.map(|token| {
				// TODO: filter
				format!("/users/{user}/roles?page={token}&limit={limit}").parse().unwrap()
			});
		
		let items = roles.into_iter()
			.map(Role::from)
			.map(Reference::from)
			.collect();
		
		Ok(Page { items, next, previous: None })
	}
	
	pub async fn list_user_factions(
		&self,
		user: &UserSpecifier, 
		page: UserFactionsPageToken,
		limit: usize,
		filter: FactionFilter,
		// TODO: member filter also
	) -> DbResult<Page<UserFactionMember>> {
		let members = faction::Entity::find().select_only()
			.tbl_col_as((faction_member::Entity, faction_member::Column::Invited), "invited")
			.tbl_col_as((faction_member::Entity, faction_member::Column::Imposed), "imposed")
			.tbl_col_as((faction_member::Entity, faction_member::Column::Owner), "owner")
			.tbl_col_as((faction::Entity, faction::Column::Id), "faction_id")
			.tbl_col_as((faction::Entity, faction::Column::Name), "faction_name")
			.tbl_col_as((faction::Entity, faction::Column::Icon), "faction_icon")
			.tbl_col_as((faction::Entity, faction::Column::CreatedAt), "faction_created_at")
			.column_as(Expr::col(("member_count".into_iden(), faction_member::Column::Member)).count(), "faction_size")
			.tbl_col_as((user::Entity, user::Column::Id), "member_id")
			.tbl_col_as((user::Entity, user::Column::Subject), "member_subject")
			.tbl_col_as((user::Entity, user::Column::Name), "member_name")
			.tbl_col_as((user::Entity, user::Column::CreatedAt), "member_created_at")
			// for unknown reasons, this is already included??
			// .inner_join(faction_member::Entity)
			.inner_join(user::Entity)
			.join_as(sea_orm::JoinType::FullOuterJoin, faction::Relation::FactionMember.def(), "member_count")
			.group_by(faction::Column::Id)
			.group_by(user::Column::Id)
			.group_by(faction_member::Column::Invited)
			.group_by(faction_member::Column::Imposed)
			.group_by(faction_member::Column::Owner)
			.group_by(faction_member::Column::Faction)
			.group_by(faction_member::Column::Member)
			.apply_if(filter.name, |q, name| q.filter(faction::Column::Name.like(format!("%{name}%"))))
			.apply_if(filter.created_at.start, |q, start| {
				if filter.created_at.end.is_none() {
					q.filter(faction::Column::CreatedAt.gte(start).or(faction::Column::CreatedAt.is_null()))
				} else {
					q.filter(faction::Column::CreatedAt.gte(start))
				}
			})
			.apply_if(filter.created_at.end, |q, end| q.filter(faction::Column::CreatedAt.lte(end)))
			.apply_if(filter.size.start, |q, start| {
				let expr = SimpleExpr::Binary(
					Box::new(SimpleExpr::Column(ColumnRef::Column("size".into_iden()))),
					sea_query::BinOper::GreaterThanOrEqual,
					Box::new(SimpleExpr::Constant((start as i64).into()))
				);
				q.filter(expr)
			})
			.apply_if(filter.size.end, |q, end| {
				let expr = SimpleExpr::Binary(
					Box::new(SimpleExpr::Column(ColumnRef::Column("size".into_iden()))),
					sea_query::BinOper::SmallerThanOrEqual,
					Box::new(SimpleExpr::Constant((end as i64).into()))
				);
				q.filter(expr)
			})
			.filter(faction_member::Column::Member.eq(user))
			.filter(faction_member::Column::Faction.gte(page.0))
			.order_by(faction_member::Column::Faction, sea_orm::Order::Asc)
			.into_model::<FactionMemberFull>()
			.all(&self.connection).await?;
		
		let next = members.windows(2).nth(limit.saturating_sub(1))
			.map(|pair| &pair[0])
			.map(UserFactionsPageToken::from)
			.map(|token| {
				// TODO: filter
				format!("/users/{user}/factions?page={token}&limit={limit}").parse().unwrap()
			});
		
		let items = members.into_iter()
			.map(FactionMember::from)
			.map(UserFactionMember::from)
			.collect();
		
		Ok(Page { items, next, previous: None })
	}
	
	pub async fn create_role_member(
		&self,
		user: &UserSpecifier,
		role: &RoleSpecifier,
	) -> DbResult<TryInsertResult<RoleMember>> {
		let role_member = role_member::ActiveModel {
			role: Set(role.0),
			member: Set(user.0),
		};
		
		let transaction = self.begin().await?;
		
		let user = match transaction.get_user(user).await? {
			Some(user) => Reference::from(user),
			None => return Ok(TryInsertResult::Empty),
		};
		
		let role = match transaction.get_role(role).await? {
			Some(role) => Reference::from(role),
			None => return Ok(TryInsertResult::Empty),
		};
		
		let insert = role_member::Entity::insert(role_member)
			.on_conflict_do_nothing()
			.exec(&transaction.connection).await?;
		
		transaction.commit().await?;
		
		match insert {
			TryInsertResult::Inserted(_) => {
				Ok(TryInsertResult::Inserted(RoleMember { user, role }))
			},
			TryInsertResult::Empty => Ok(TryInsertResult::Empty),
			TryInsertResult::Conflicted => Ok(TryInsertResult::Conflicted),
		}
	}
	
	pub async fn delete_role_member(
		&self,
		user: &UserSpecifier,
		role: &RoleSpecifier,
	) -> DbResult<Option<()>> {
		let role_member = role_member::ActiveModel {
			role: Set(role.0),
			member: Set(user.0),
		};
		
		let delete = role_member::Entity::delete(role_member)
			.exec(&self.connection).await?;
		
		match delete.rows_affected {
			0 => Ok(None),
			1 => Ok(Some(())),
			_ => panic!("deleted multiple role members with the same keys"),
		}
	}
	
	pub async fn list_faction_members(
		&self,
		faction: &FactionSpecifier,
		page: FactionMembersPageToken,
		limit: usize,
		filter: FactionMemberFilter,
	) -> DbResult<Page<Reference<FactionMember>>> {
		let members = faction_member::Entity::find().select_only()
			.tbl_col_as((faction_member::Entity, faction_member::Column::Invited), "invited")
			.tbl_col_as((faction_member::Entity, faction_member::Column::Imposed), "imposed")
			.tbl_col_as((faction_member::Entity, faction_member::Column::Owner), "owner")
			.tbl_col_as((faction::Entity, faction::Column::Id), "faction_id")
			.tbl_col_as((faction::Entity, faction::Column::Name), "faction_name")
			.tbl_col_as((faction::Entity, faction::Column::Icon), "faction_icon")
			.tbl_col_as((faction::Entity, faction::Column::CreatedAt), "faction_created_at")
			.column_as(Expr::col(("member_count".into_iden(), faction_member::Column::Member)).count(), "size")
			.tbl_col_as((user::Entity, user::Column::Id), "member_id")
			.tbl_col_as((user::Entity, user::Column::Subject), "member_subject")
			.tbl_col_as((user::Entity, user::Column::Name), "member_name")
			.tbl_col_as((user::Entity, user::Column::CreatedAt), "member_created_at")
			.inner_join(faction::Entity)
			.inner_join(user::Entity)
			.filter(faction_member::Column::Member.gt(page.0))
			.order_by(faction_member::Column::Member, sea_orm::Order::Asc)
			.apply_if(filter.owner, |q, owner| q.filter(faction_member::Column::Owner.eq(owner)))
			.join_as(sea_orm::JoinType::FullOuterJoin, faction::Relation::FactionMember.def(), "member_count")
			.group_by(Expr::col(("member_count".into_iden(), faction_member::Column::Member)))
			.filter(faction_member::Column::Faction.eq(faction))
			.into_model::<FactionMemberFull>()
			.all(&self.connection).await?;
		
		let next = members.windows(2).nth(limit.saturating_sub(1))
			.map(|pair| &pair[0])
			.map(FactionMembersPageToken::from)
			.map(|token| {
				// TODO: filter
				format!("/factions/{faction}/members?page={token}&limit={limit}").parse().unwrap()
			});
		
		let items = members.into_iter()
			.map(FactionMember::from)
			.map(Reference::from)
			.collect();
		
		Ok(Page { items, next, previous: None })
	}
	
	pub async fn all_faction_members(
		&self,
		faction: &FactionSpecifier,
	) -> DbResult<Vec<UserSpecifier>> {
		let members = faction_member::Entity::find()
			.filter(faction_member::Column::Faction.eq(faction))
			.all(&self.connection).await?;
		
		Ok(members.into_iter().map(|m| UserSpecifier(m.member)).collect())
	}
	
	pub async fn all_faction_owners(
		&self,
		faction: &FactionSpecifier,
	) -> DbResult<Vec<UserSpecifier>> {
		let members = faction_member::Entity::find()
			.filter(faction_member::Column::Faction.eq(faction))
			.filter(faction_member::Column::Owner.eq(true))
			.all(&self.connection).await?;
		
		Ok(members.into_iter().map(|m| UserSpecifier(m.member)).collect())
	}
	
	pub async fn get_faction_member(
		&self,
		faction: &FactionSpecifier,
		member: &UserSpecifier,
	) -> DbResult<Option<FactionMember>> {
		let transaction = self.begin().await?;
		
		let find = faction_member::Entity::find()
			.filter(faction_member::Column::Member.eq(member))
			.filter(faction_member::Column::Faction.eq(faction))
			.one(&transaction.connection).await?;
		
		if let Some(model) = find {
			let faction = self.get_faction(faction).await?
				.expect("failed to find faction for faction member");
			
			let user = self.get_user(member).await?
				.expect("failed to find user for faction member");
			
			transaction.commit().await?;
			
			Ok(Some(FactionMember::from_parts(model, faction, user)))
		} else {
			Ok(None)
		}
		
		// Doesn't work, was kinda a crazy idea. Might be possible, but probably not worth it
		
		// let member = faction_member::Entity::find().select_only()
		// 	.tbl_col_as((faction_member::Entity, faction_member::Column::Invited), "invited")
		// 	.tbl_col_as((faction_member::Entity, faction_member::Column::Imposed), "imposed")
		// 	.tbl_col_as((faction_member::Entity, faction_member::Column::Owner), "owner")
		// 	.tbl_col_as((faction::Entity, faction::Column::Id), "faction_id")
		// 	.tbl_col_as((faction::Entity, faction::Column::Name), "faction_name")
		// 	.tbl_col_as((faction::Entity, faction::Column::Icon), "faction_icon")
		// 	.tbl_col_as((faction::Entity, faction::Column::CreatedAt), "faction_created_at")
		// 	.column_as(Expr::col(("member_count".into_iden(), faction_member::Column::Member)).count(), "size")
		// 	.tbl_col_as((user::Entity, user::Column::Id), "member_id")
		// 	.tbl_col_as((user::Entity, user::Column::Subject), "member_subject")
		// 	.tbl_col_as((user::Entity, user::Column::Name), "member_name")
		// 	.tbl_col_as((user::Entity, user::Column::CreatedAt), "member_created_at")
		// 	.inner_join(faction::Entity)
		// 	.inner_join(user::Entity)
		// 	.filter(faction_member::Column::Member.eq(member))
		// 	.filter(faction_member::Column::Faction.eq(faction))
		// 	.join_as(sea_orm::JoinType::FullOuterJoin, faction::Relation::FactionMember.def(), "member_count")
		// 	.group_by(Expr::col(("member_count".into_iden(), faction_member::Column::Member)))
		// 	.into_model::<FactionMemberFull>()
		// 	.one(&self.connection).await?;
	}
	
	pub async fn create_faction_member(
		&self,
		faction: &FactionSpecifier,
		member: &UserSpecifier,
		owner: bool,
		invited: bool,
		imposed: bool,
	) -> DbResult<TryInsertResult<FactionMember>> {
		let faction_member = faction_member::ActiveModel {
			faction: Set(faction.0),
			member: Set(member.0),
			owner: Set(owner),
			invited: Set(invited),
			imposed: Set(imposed),
		};
		
		let transaction = self.begin().await?;
		
		let user = match transaction.get_user(member).await? {
			Some(user) => Reference::from(user),
			None => return Ok(TryInsertResult::Empty),
		};
		
		let faction = match transaction.get_faction(faction).await? {
			Some(faction) => faction,
			None => return Ok(TryInsertResult::Empty),
		};
		
		let insert = faction_member::Entity::insert(faction_member)
			.on_conflict_do_nothing()
			.exec(&transaction.connection).await?;
		
		transaction.commit().await?;
		
		match insert {
			TryInsertResult::Inserted(_) => {				
				
				let faction_member = FactionMember {
					faction,
					user,
					join_intent: JoinIntent {
						faction: invited,
						member: imposed,
					},
					owner,
				};
				Ok(TryInsertResult::Inserted(faction_member))
			},
			TryInsertResult::Empty => Ok(TryInsertResult::Empty),
			TryInsertResult::Conflicted => Ok(TryInsertResult::Conflicted),
		}
	}
	
	pub async fn update_faction_member(
		&self,
		faction: &FactionSpecifier,
		member: &UserSpecifier,
		owner: Option<bool>,
		invited: Option<bool>,
		imposed: Option<bool>,
	) -> DbResult<Option<FactionMember>> {
		let faction_member = faction_member::ActiveModel { 
			faction: Set(faction.0),
			member: Set(member.0),
			owner: owner.map(Set).unwrap_or(NotSet),
			invited: invited.map(Set).unwrap_or(NotSet),
			imposed: imposed.map(Set).unwrap_or(NotSet),
		};
		
		let transaction = self.begin().await?;
		// TODO: this doesn't seem right, but just update has no fail state for not found it seems
		let update = faction_member::Entity::update_many()
			.set(faction_member)
			.filter(faction_member::Column::Faction.eq(faction))
			.filter(faction_member::Column::Member.eq(member))
			.exec_with_returning(&transaction.connection).await?;
		
		match update.as_slice() {
			[] => Ok(None),
			[faction_member] => {
				let faction = transaction.get_faction(faction).await?
					.expect("updated faction member but failed to find faction");
				let user = transaction.get_user(member).await?
					.expect("updated faction member but failed to find user");
				transaction.commit().await?;
				let faction_member = FactionMember {
					faction,
					user: Reference::from(user),
					join_intent: JoinIntent {
						faction: faction_member.invited,
						member: faction_member.imposed,
					},
					owner: faction_member.owner,
				};
				Ok(Some(faction_member))
			},
			_ => panic!("updated multiple factions with the same id"),
		}
	}
	
	pub async fn delete_faction_member(
		&self,
		faction: &FactionSpecifier,
		member: &UserSpecifier,
	) -> DbResult<Option<FactionMember>> {
		let transaction = self.begin().await?;
		
		let faction_member = transaction.get_faction_member(faction, member).await?;
		
		let mut faction_member = match faction_member {
			Some(mut member) => {
				member.join_intent.faction = false;
				member.join_intent.member = false;
				member
			},
			None => return Ok(None),
		};
		
		let delete = faction_member::Entity::delete_many()
			.filter(faction_member::Column::Faction.eq(faction))
			.filter(faction_member::Column::Member.eq(member))
			.exec(&transaction.connection).await?;
		
		faction_member.faction.size -= 1;
		
		transaction.commit().await?;
		
		match delete.rows_affected {
			0 => Ok(None),
			1 => Ok(Some(faction_member)),
			_ => panic!("deleted multiple faction members with the same keys"),
		}
	}
}
