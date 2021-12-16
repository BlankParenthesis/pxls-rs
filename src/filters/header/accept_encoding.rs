use super::*;

pub fn gzip() -> impl Filter<Extract = (), Error = Rejection> + Copy {
	warp::header("accept-encoding")
		.and_then(|header: String| {
			async move {
				header
					.split(',')
					.any(|value| {
						let value = value.split(';').next().unwrap_or("");
						value.trim() == "gzip"
					})
					.then(|| ())
					.ok_or(warp::reject())
			}
		})
		.untuple_one()
}
