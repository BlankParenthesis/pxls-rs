use std::time::{SystemTime, UNIX_EPOCH, Duration};

use enumset::EnumSet;
use futures_util::future;
use jsonwebtoken::TokenData;
use warp::http::{header, StatusCode};
use warp::{reject::Reject, Filter, Rejection, Reply};

use crate::openid::Identity;
use crate::permissions::Permission;
use crate::openid::{self, ValidationError};

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
			Self::Invalid => StatusCode::BAD_REQUEST,
			Self::MissingScheme => StatusCode::BAD_REQUEST,
			Self::MissingToken => StatusCode::BAD_REQUEST,
			Self::ValidationError(_) => StatusCode::UNAUTHORIZED,
		}
		.into_response()
	}
}
impl Reject for BearerError {}

pub fn bearer() -> impl Filter<Extract = (Option<Bearer>,), Error = Rejection> + Copy {
	warp::any()
		.and(warp::header::<String>(header::AUTHORIZATION.as_str()).map(Some))
		.recover(|_| -> future::Ready<Result<_, Rejection>> { future::ok(None) })
		.unify()
		.and_then(|header_value: Option<String>| async move {
			if let Some(header_value) = header_value {
				let mut parts = header_value.as_str().splitn(2, ' ');
				match parts.next() {
					Some("Bearer") => {
						parts.next()
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
		})
		.and_then(|token: Option<String>| async move {
			if let Some(token) = token {
				validator(token).await
					.map(Some)
					.map_err(warp::reject::custom)
			} else {
				Ok(None)
			}
		})
}

pub async fn validator(token: String) -> Result<Bearer, BearerError> {
	openid::validate_token(&token).await
		.map(Bearer::from)
		.map_err(BearerError::ValidationError)
}


#[derive(Debug)]
pub enum PermissionsError {
	MissingPermission(Permission),
}
impl Reject for PermissionsError {}

#[derive(Debug)]
pub struct Bearer {
	valid_until: SystemTime,
	pub id: String,
}

impl From<TokenData<Identity>> for Bearer {
	fn from(token_data: TokenData<Identity>) -> Self {
		Self {
			valid_until: UNIX_EPOCH + Duration::from_secs(token_data.claims.exp),
			id: token_data.claims.sub,
		}
	}
}

impl Bearer {
	pub fn permissions(&self) -> EnumSet<Permission> {
		// TODO: obtain user permissions
		EnumSet::all()
	}

	pub fn is_valid(&self) -> bool {
		SystemTime::now() < self.valid_until 
	}
}

pub fn with_permission(
	permission: Permission
) -> (impl Fn(Option<Bearer>) -> future::Ready<Result<Option<Bearer>, Rejection>> + Clone) {
	move |bearer| {
		let permissions = match bearer {
			Some(ref bearer) => bearer.permissions(),
			None => Permission::defaults(),
		};

		if permissions.contains(permission) {
			future::ok(bearer)
		} else {
			let error = PermissionsError::MissingPermission(permission);
			future::err(warp::reject::custom(error))
		}
	}
}
