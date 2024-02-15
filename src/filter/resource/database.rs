use std::sync::Arc;

use warp::{Filter, reject, Rejection};

use crate::database::Database;

#[derive(Debug)]
struct DbConnectionFailed;
impl reject::Reject for DbConnectionFailed {}

pub fn connection<T: Database>(
	db: Arc<T>,
) -> impl Filter<Extract = (T::Connection,), Error = Rejection> + Clone {
	warp::any().and_then(move || {
		let db = db.clone();
		async move {
			db.connection().await
				.map_err(|_| DbConnectionFailed)
				.map_err(reject::custom)
		}
	})
}
