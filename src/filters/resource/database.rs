use std::sync::Arc;

use super::*;
use sea_orm::DatabaseConnection as Connection;

pub fn connection(
	db: Arc<Connection>
) -> impl Filter<Extract = (Arc<Connection>,), Error = Infallible> + Clone {
	warp::any().map(move || db.clone())
}
