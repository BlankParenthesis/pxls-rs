use futures_util::future;
use http::{StatusCode, header};
use warp::{reject::Reject, Filter, Rejection, Reply};

use crate::authentication::openid::ValidationError;
use crate::objects::AuthedUser;

#[derive(Debug)]
pub enum BearerError {
	Invalid,
	MissingScheme,
	MissingToken,
	ValidationError(ValidationError),
}

impl Reply for BearerError {
	fn into_response(self) -> warp::reply::Response {
		match self {
			BearerError::Invalid => StatusCode::BAD_REQUEST,
			BearerError::MissingScheme => StatusCode::BAD_REQUEST,
			BearerError::MissingToken => StatusCode::BAD_REQUEST,
			BearerError::ValidationError(_) => StatusCode::UNAUTHORIZED,
		}
		.into_response()
	}
}
impl Reject for BearerError {}

pub fn bearer() -> impl Filter<Extract = (AuthedUser,), Error = Rejection> + Copy {
	warp::any()
		.and(warp::header::<String>(header::AUTHORIZATION.as_str()).map(Some))
		.recover(|_| -> future::Ready<Result<_, Rejection>> { future::ok(None) })
		.unify()
		.and_then(|header_value: Option<String>| {
			async move {
				if let Some(header_value) = header_value {
					let mut parts = header_value.as_str().splitn(2, ' ');
					match parts.next() {
						Some("Bearer") => {
							parts
								.next()
								.ok_or(BearerError::MissingToken)
								.map(String::from)
								.map(Some)
						},
						Some(_) => Err(BearerError::Invalid),
						None => Err(BearerError::MissingScheme),
					}
				} else {
					Ok(None)
				}
				.map_err(warp::reject::custom)
			}
		})
		.and_then(|token: Option<String>| {
			async move {
				if let Some(token) = token {
					validator(token)
						.await
						.map_err(warp::reject::custom)
				} else {
					Ok(AuthedUser::None)
				}
			}
		})
}

pub async fn validator(token: String) -> Result<AuthedUser, BearerError> {
	crate::authentication::openid::validate_token(&token)
		.await
		.map(AuthedUser::from)
		.map_err(BearerError::ValidationError)
}
