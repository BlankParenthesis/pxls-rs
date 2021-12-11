use super::*;

pub fn get() -> impl Filter<Extract = impl Reply, Error = Rejection> + Clone {
	warp::path("access")
		.and(warp::path::end())
		.and(warp::get())
		.and(authorization::bearer().map(User::from))
		.map(|user: User| {
			json(
				&Option::<User>::from(user)
					.unwrap_or_default()
					.permissions,
			)
		})
}
