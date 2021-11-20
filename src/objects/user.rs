use std::collections::HashSet;
use actix_web::{FromRequest, HttpRequest, dev::Payload};
use futures_util::future::{Ready, ok};
use serde::Deserialize;

use crate::access::permissions::Permission;

#[derive(Debug, Clone)]
pub struct User {
	pub id: Option<String>,
	pub permissions: HashSet<Permission>,
}

impl Default for User {
	fn default() -> Self {
		let mut permissions = HashSet::new();
		permissions.insert(Permission::Info);
		permissions.insert(Permission::BoardsList);
		permissions.insert(Permission::BoardsGet);
		permissions.insert(Permission::BoardsPost);
		permissions.insert(Permission::BoardsPatch);
		permissions.insert(Permission::BoardsDelete);
		permissions.insert(Permission::BoardsDataGet);
		permissions.insert(Permission::BoardsDataPatch);
		permissions.insert(Permission::BoardsUsers);
		permissions.insert(Permission::BoardsPixelsList);
		permissions.insert(Permission::BoardsPixelsGet);
		permissions.insert(Permission::SocketCore);
		
		Self {
			id: None,
			permissions,
		}
	}
}

pub enum AuthedUser {
	Authed(User),
	None,
}

impl From<AuthedUser> for Option<User> {
    fn from(authed: AuthedUser) -> Self {
        match authed {
			AuthedUser::Authed(user) => Some(user),
			AuthedUser::None => None,
		}
    }
}

impl From<Option<User>> for AuthedUser {
    fn from(user: Option<User>) -> Self {
        match user {
			Some(user) => AuthedUser::Authed(user),
			None => AuthedUser::None,
		}
    }
}

#[derive(Deserialize)]
struct UserInfo {
	sub: String,
}

impl FromRequest for AuthedUser {
	type Error = actix_web::Error;
	type Future = Ready<Result<Self, Self::Error>>;
	type Config = ();
	
	fn from_request(req: &HttpRequest, _payload: &mut Payload) -> Self::Future {
		ok(req.extensions().get::<User>().cloned().into())
	}
}
