use std::time::Duration;

use migration::{Migrator, MigratorTrait};
use reqwest::StatusCode;
use sea_orm::{ConnectOptions, ConnectionTrait, DatabaseConnection, DatabaseTransaction, StreamTrait, TransactionTrait};
use warp::{reject::Reject, reply::Reply};

use crate::config::CONFIG;

mod entities;
mod specifier;
mod filter;

mod board;
mod color;
mod ban;
mod user;
mod role;
mod role_member;
mod faction;
mod faction_member;
mod placement;
mod sector;
mod report;
mod notice;
mod board_notice;

pub use specifier::{Specifier, SpecfierParseError};
pub use ban::{Ban, BanSpecifier, BanListSpecifier};
pub use user::{User, UserSpecifier, UserStatsSpecifier};
pub use board::{BoardInfo, BoardSpecifier, BoardListSpecifier};
pub use board_notice::{BoardsNotice, BoardNoticeSpecifier, BoardNoticeListSpecifier};
pub use color::{Palette, Color};
pub use placement::{Placement, PlacementSpecifier, PlacementPageToken, PlacementListSpecifier};
pub use sector::{Sector, SectorBuffer, MaskValue, Change, BufferRead};
pub use role::{Role, RoleSpecifier, UserRolesListSpecifier};
pub use notice::{Notice, NoticeSpecifier, NoticeFilter};
pub use report::{Report, ReportSpecifier, ReportFilter, ReportPageToken, ReportStatus, Artifact, ReportHistorySpecifier};
pub use faction::{Faction, FactionSpecifier, FactionListSpecifier};
pub use faction_member::{FactionMember, FactionMemberSpecifier, FactionMemberListSpecifier, UserFactionMemberListSpecifier, FactionMemberCurrentSpecifier};

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
	fn from(value: DatabaseError) -> Self {
		StatusCode::from(&value)
	}
}

impl Reply for DatabaseError {
	fn into_response(self) -> warp::reply::Response {
		StatusCode::from(self).into_response()
	}
}

impl Reject for DatabaseError {}

pub type DbResult<T> = Result<T, DatabaseError>;
pub type DbInsertResult<T> = Result<T, InsertError>;

#[derive(Clone, Copy)]
pub enum Order { Forward, Reverse }

pub struct Connection<C: TransactionTrait + ConnectionTrait + StreamTrait> {
	connection: C,
}

pub type DbConn = Connection<DatabaseConnection>;

impl Connection<DatabaseTransaction> {
	pub async fn commit(self) -> DbResult<()> {
		self.connection.commit().await
			.map_err(DatabaseError::from)
	}
}

impl<C: TransactionTrait + ConnectionTrait + StreamTrait> Connection<C> {
	pub async fn begin(&self) -> DbResult<Connection<DatabaseTransaction>> {
		self.connection.begin().await
			.map(|connection| Connection { connection })
			.map_err(DatabaseError::from)
	}
}

pub struct Database {
	pool: DatabaseConnection,
}

impl Database {
	pub async fn connect() -> Result<Self, DatabaseError> {
		let url = CONFIG.database_url.to_string();
		let mut connect_options = ConnectOptions::new(url);
		connect_options
			.connect_timeout(Duration::from_secs(2))
			.acquire_timeout(Duration::from_secs(2));
		
		let pool = sea_orm::Database::connect(connect_options).await?;
		Migrator::up(&pool, None).await?;
		Ok(Self { pool })
	}

	pub async fn connection(&self) -> Result<Connection<DatabaseConnection>, DatabaseError> {
		let connection = self.pool.clone();
		Ok(Connection { connection })
	}
}

pub enum InsertError {
	DatabaseError(DatabaseError),
	MissingDependency,
	AlreadyExists,
}

impl From<&InsertError> for StatusCode {
	fn from(value: &InsertError) -> Self {
		match value {
			InsertError::DatabaseError(e) => e.into(),
			InsertError::MissingDependency => StatusCode::NOT_FOUND,
			InsertError::AlreadyExists => StatusCode::CONFLICT,
		}
	}
}

impl From<InsertError> for StatusCode {
	fn from(value: InsertError) -> Self {
		StatusCode::from(&value)
	}
}

impl From<DatabaseError> for InsertError {
	fn from(value: DatabaseError) -> Self {
		InsertError::DatabaseError(value)
	}
}

impl From<sea_orm::DbErr> for InsertError {
	fn from(value: sea_orm::DbErr) -> Self {
		InsertError::DatabaseError(value.into())
	}
}
