use futures_util::future;
use warp::{Filter, Rejection};

fn has_gzip(header: &str) -> bool {
	header.split(',')
		.any(|value| {
			let value = value.split(';').next().unwrap_or("");
			value.trim() == "gzip"
		})
}

pub fn gzip_opt() -> impl Filter<Extract = (bool,), Error = Rejection> + Copy {
	warp::header::optional("accept-encoding")
		.map(|header: Option<String>| {
			header.map(|h| has_gzip(&h)).unwrap_or(false)
		})
}

pub fn gzip() -> impl Filter<Extract = (), Error = Rejection> + Copy {
	warp::header("accept-encoding")
		.and_then(|header: String| async move {
			has_gzip(&header)
				.then_some(())
				.ok_or_else(warp::reject)
		})
		// Convert accept-encoding rejection into 404 rejection.
		// Without this, the encoding error can fall through and clients
		// that don't accept gzip get this instead of 404.
		.recover(|_| -> future::Ready<Result<_, Rejection>> {
			future::err(warp::reject())
		})
		.unify()
		.untuple_one()
}
