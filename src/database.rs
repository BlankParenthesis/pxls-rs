use sea_orm::DatabaseConnection;
use reqwest::StatusCode;
use sea_orm::DbErr;
use thiserror::Error;
use warp::Reply;
use warp::reply;

mod users;
mod boards;

pub use users::{
	UsersDatabase,
	UsersConnection,
	FetchError,
	CreateError,
	UpdateError,
	DeleteError,
	Role,
};
pub use boards::BoardsDatabase;

pub type BoardsConnection = boards::BoardsConnection<DatabaseConnection>;
pub type BoardsConnectionGeneric<T> = boards::BoardsConnection<T>;

pub enum Order { Forward, Reverse }

#[derive(Error, Debug)]
pub enum DatabaseError<T> {
    #[error(transparent)]
	DbErr(DbErr),
    #[error(transparent)]
	Other(#[from] T),
}

impl<T: Send + Sync + Reply> Reply for DatabaseError<T> {
    fn into_response(self) -> reply::Response {
		match self {
			DatabaseError::DbErr(err) => {
				StatusCode::INTERNAL_SERVER_ERROR.into_response()
			},
			DatabaseError::Other(other) => other.into_response(),
		}
    }
}

#[async_trait::async_trait]
pub trait Database: Sized + Send + Sync {
	type Error: std::fmt::Debug;
	type Connection: Send + Sync;

	async fn connect() -> Result<Self, Self::Error>;
	async fn connection(&self) -> Result<Self::Connection, Self::Error>;
}