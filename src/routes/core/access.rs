use std::sync::Arc;

use warp::reject::Rejection;
use warp::{Reply, Filter, reply::json};

use crate::filter::header::authorization::{self, Bearer, UsersDBError};
use crate::filter::resource::database;
use crate::permissions::Permission;
use crate::database::{UsersConnection, UsersDatabase};

pub fn get(
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("access")
		.and(warp::path::end())
		.and(warp::get())
		.and(authorization::bearer())
		.and(database::connection(users_db))
		.and_then(|user: Option<Bearer>, mut connection: UsersConnection| async move {
			let permissions = match user {
				Some(user) => {
					connection.user_permissions(&user.id).await
						.map_err(UsersDBError::Raw)
						.map_err(Rejection::from)?
				},
				None => Permission::defaults(),
			};
			
			let data = json(&permissions.into_iter().collect::<Vec<_>>());

			Ok::<_, Rejection>(data)
		})
}
