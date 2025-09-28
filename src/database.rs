use sea_orm::DatabaseConnection;

mod boards;

pub use boards::{
	BoardsDatabase,
	DatabaseError,
	Faction,
	FactionSpecifier,
	FactionMember,
	Role,
	RoleSpecifier,
	User,
	UserSpecifier,
	Ban,
	BanSpecifier,
};

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
