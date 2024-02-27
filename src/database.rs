use sea_orm::DatabaseConnection;

mod users;
mod boards;

pub use users::{
	UsersDatabase,
	UsersConnection,
	Role,
	UsersDatabaseError,
	LdapPageToken,
};
pub use boards::{BoardsDatabase, BoardsDatabaseError};

pub type BoardsConnection = boards::BoardsConnection<DatabaseConnection>;
pub type BoardsConnectionGeneric<T> = boards::BoardsConnection<T>;

pub enum Order { Forward, Reverse }

#[async_trait::async_trait]
pub trait Database: Sized + Send + Sync {
	type Error: std::fmt::Debug;
	type Connection: Send + Sync;

	async fn connect() -> Result<Self, Self::Error>;
	async fn connection(&self) -> Result<Self::Connection, Self::Error>;
}