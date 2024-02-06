use std::sync::Arc;
use warp::{Filter, reject::Reject, Rejection};

use crate::database::users::LDAPConnectionManager;

type Pool = deadpool::managed::Pool<LDAPConnectionManager>;
type Connection = deadpool::managed::Object<LDAPConnectionManager>;

#[derive(Debug)]
pub struct LDAPConnectionFailed;
impl Reject for LDAPConnectionFailed {}

pub fn connection(
	pool: &Arc<Pool>,
) -> impl Filter<Extract = (Connection,), Error = Rejection> + Clone {
	let pool = pool.clone();
	warp::any().and_then(move || {
		let pool = pool.clone();
		async move {
			pool.get().await
				.map_err(|_| LDAPConnectionFailed)
				.map_err(warp::reject::custom)
		}
	})
}
