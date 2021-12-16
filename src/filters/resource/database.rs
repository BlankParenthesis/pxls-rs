use super::*;
use crate::database::{Connection, Pool};

pub fn connection(
	pool: std::sync::Arc<Pool>
) -> impl Filter<Extract = (Connection,), Error = Infallible> + Clone {
	warp::any().map(move || pool.get().unwrap())
}
