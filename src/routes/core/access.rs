use std::sync::Arc;

use enumset::EnumSet;
use warp::reject::Rejection;
use warp::{Reply, Filter};

use crate::database::BoardsDatabase;
use crate::filter::header::authorization;
use crate::permissions::Permission;

pub fn get(
	db: Arc<BoardsDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("access")
		.and(warp::path::end())
		.and(warp::get())
		.and(authorization::permissions(db))
		.map(|permissions: EnumSet<Permission>, _, _| {
			let permissions = permissions.into_iter().collect::<Vec<_>>();
			warp::reply::json(&permissions)
		})
}
