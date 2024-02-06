pub type DbResult<T> = Result<T, sea_orm::DbErr>;

//pub mod model;
pub mod queries;
//pub mod schema;
pub mod entities;
pub mod migrations;
