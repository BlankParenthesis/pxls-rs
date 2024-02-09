use warp::{Reply, Rejection, Filter, reply::json};

use crate::{filter::header::authorization::{self, Bearer}, permissions::Permission};

pub fn get() -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("access")
		.and(warp::path::end())
		.and(warp::get())
		.and(authorization::bearer())
		.map(|user: Option<Bearer>| {
			let permissions = match user {
				Some(bearer) => bearer.permissions(),
				None => Permission::defaults(),
			};
			
			json(&permissions)
		})
}
