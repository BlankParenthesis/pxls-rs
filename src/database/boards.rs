pub mod entities;
pub mod migrations;

use reqwest::StatusCode;
use sea_orm::DbErr;
use thiserror::Error;
use warp::Reply;

pub type DbResult<T> = Result<T, sea_orm::DbErr>;

#[derive(Error, Debug)]
pub enum DatabaseError<T> {
    #[error(transparent)]
	DbErr(DbErr),
    #[error(transparent)]
	Other(#[from] T),
}

impl<T: Send + Sync + Reply> Reply for DatabaseError<T> {
    fn into_response(self) -> warp::reply::Response {
		match self {
			DatabaseError::DbErr(_) => {
				StatusCode::INTERNAL_SERVER_ERROR.into_response()
			},
			DatabaseError::Other(other) => other.into_response(),
		}
    }
}