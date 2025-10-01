use std::time::{SystemTime, UNIX_EPOCH, Duration};
use std::sync::Arc;

use enumset::EnumSet;
use futures_util::future;
use jsonwebtoken::TokenData;
use warp::http::{header, StatusCode};
use warp::{reject::Reject, Filter, Rejection, Reply};

use crate::openid::Identity;
use crate::openid::{self, ValidationError};
use crate::permissions::Permission;
use crate::database::{DbConn, Database, DatabaseError, User, UserSpecifier};
use crate::filter::resource::database;

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
			Self::ValidationError(ValidationError::JWTError(_),) => StatusCode::UNAUTHORIZED,
			Self::ValidationError(ValidationError::DiscoveryError(_)) => StatusCode::INTERNAL_SERVER_ERROR,
			Self::ValidationError(ValidationError::NoValidKeys) => StatusCode::INTERNAL_SERVER_ERROR,
		}
		.into_response()
	}
}
impl Reject for BearerError {}

pub fn bearer() -> impl Filter<Extract = (Option<Bearer>,), Error = Rejection> + Copy {
	warp::any()
		.and(warp::header::<String>(header::AUTHORIZATION.as_str()).map(Some))
		.recover(|_| future::ok::<_, Rejection>(None))
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
	MissingPermission
}
impl Reject for PermissionsError {}

#[derive(Debug)]
pub struct Bearer {
	valid_until: SystemTime,
	subject: String,
	username: String,
}

impl Bearer {
	pub async fn user(
		&self,
		connection: &DbConn,
	) -> Result<User, DatabaseError> {
		connection.create_user(
			self.subject.clone(),
			self.username.clone(),
			SystemTime::now(),
		).await
	}
}

impl From<TokenData<Identity>> for Bearer {
	fn from(token_data: TokenData<Identity>) -> Self {
		Self {
			valid_until: UNIX_EPOCH + Duration::from_secs(token_data.claims.exp),
			subject: token_data.claims.sub,
			username: token_data.claims.preferred_username,
		}
	}
}

impl Bearer {
	pub fn into_user(self, user: &User) -> AuthenticatedUser {
		debug_assert!(user.subject == self.subject);
		AuthenticatedUser {
			user: *user.specifier(),
			valid_until: self.valid_until,
		}
	}
}

#[derive(Debug)]
pub struct AuthenticatedUser {
	user: UserSpecifier,
	valid_until: SystemTime,
}

impl AuthenticatedUser {
	pub fn is_valid(&self) -> bool {
		SystemTime::now() < self.valid_until 
	}
	
	pub fn specifier(&self) -> &UserSpecifier {
		&self.user
	}
}


pub fn permissions(db: Arc<Database>) -> impl Filter<
	Extract = (EnumSet<Permission>, Option<User>, DbConn),
	Error = Rejection,
> + Clone {
	warp::any()
		.and(bearer())
		.and(database::connection(db))
		.and_then(|bearer: Option<Bearer>, connection: DbConn| async {
			match bearer {
				Some(bearer) => {
					let user = bearer.user(&connection).await?;
					let permissions = connection.user_permissions(user.specifier()).await?;
					Ok((permissions, Some(user), connection))
				},
				None => {
					let permissions = connection.anonymous_permissions().await?;
					Ok::<_, Rejection>((permissions, None, connection))
				},
			}
		})
		.untuple_one()
}

pub fn has_permissions(
	user_permissions: EnumSet<Permission>,
	permissions: EnumSet<Permission>,
) -> bool {
	user_permissions.is_superset(permissions)
}

pub fn has_permissions_current(
	user_permissions: EnumSet<Permission>,
	permissions: EnumSet<Permission>,
) -> bool {
	for permission in permissions.into_iter() {
		let has_regular = user_permissions.contains(permission);
		let has_current = user_permissions.contains(permission.to_current().unwrap());
		if !has_regular && !has_current {
			return false;
		}
	}
	
	true
}

pub fn authorized(
	db: Arc<Database>,
	permissions: EnumSet<Permission>,
) -> impl Filter<
	Extract = (Option<User>, DbConn),
	Error = Rejection,
> + Clone {
	warp::any()
		.and(self::permissions(db))
		.and_then(move |user_permissions: EnumSet<Permission>, user: Option<User>, connection| async move {
			
			if !has_permissions(user_permissions, permissions) {
				let error = PermissionsError::MissingPermission;
				return Err(Rejection::from(error));
			}
			Ok((user, connection))
		})
		.untuple_one()
}
