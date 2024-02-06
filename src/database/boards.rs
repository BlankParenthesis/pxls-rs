pub type DbResult<T> = Result<T, sea_orm::DbErr>;

pub mod entities;
pub mod migrations;