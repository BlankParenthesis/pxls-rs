use std::sync::Arc;

use enumset::EnumSet;
use warp::reject::Rejection;
use warp::{Reply, Filter};

use crate::filter::header::authorization;
use crate::database::UsersDatabase;
use crate::permissions::Permission;

pub fn get(
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("access")
		.and(warp::path::end())
		.and(warp::get())
		.and(authorization::permissions(users_db))
		.map(|permissions: EnumSet<Permission>, _, _| {
			let permissions = permissions.into_iter().collect::<Vec<_>>();
			warp::reply::json(&permissions)
		})
}
