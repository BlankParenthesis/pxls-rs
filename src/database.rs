use reqwest::StatusCode;
use sea_orm::DatabaseConnection;

mod users;
mod boards;

pub use users::{
	UsersDatabase,
	UsersConnection,
	User,
	Role,
	UsersDatabaseError,
	LdapPageToken,
	Faction,
	FactionMember,
	JoinIntent,
};
pub use boards::{BoardsDatabase, BoardsDatabaseError};
use warp::{reject::Reject, reply::Reply};

pub type BoardsConnection = boards::BoardsConnection<DatabaseConnection>;
pub type BoardsConnectionGeneric<T> = boards::BoardsConnection<T>;

#[derive(Clone, Copy)]
pub enum Order { Forward, Reverse }

#[async_trait::async_trait]
pub trait Database: Sized + Send + Sync {
	type Error: std::fmt::Debug;
	type Connection: Send + Sync;

	async fn connect() -> Result<Self, Self::Error>;
	async fn connection(&self) -> Result<Self::Connection, Self::Error>;
}

#[derive(Debug)]
pub enum DatabaseError {
	Users(UsersDatabaseError),
	Boards(BoardsDatabaseError),
}

impl From<UsersDatabaseError> for DatabaseError {
	fn from(value: UsersDatabaseError) -> Self {
		Self::Users(value)
	}
}

impl From<BoardsDatabaseError> for DatabaseError {
	fn from(value: BoardsDatabaseError) -> Self {
		Self::Boards(value)
	}
}

impl Reject for DatabaseError {}

impl From<DatabaseError> for StatusCode {
	fn from(value: DatabaseError) -> Self {
		match value {
			DatabaseError::Users(u) => u.into(),
			DatabaseError::Boards(b) => b.into(),
		}
	}
}

impl Reply for DatabaseError {
	fn into_response(self) -> warp::reply::Response {
		match self {
			DatabaseError::Users(u) => u.into_response(),
			DatabaseError::Boards(b) => b.into_response(),
		}
	}
}