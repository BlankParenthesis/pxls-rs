use futures_util::future;
use serde::{Serialize, Serializer};
use warp::{reject::Reject, Rejection};

use crate::objects::{AuthedUser, User};

#[derive(PartialEq, Eq, Hash, Debug, Clone, Copy)]
pub enum Permission {
	Info,
	BoardsList,
	BoardsGet,
	BoardsPost,
	BoardsPatch,
	BoardsDelete,
	BoardsDataGet,
	BoardsDataPatch,
	BoardsUsers,
	BoardsPixelsList,
	BoardsPixelsGet,
	BoardsPixelsPost,
	SocketCore,
	SocketAuthentication,
}

impl Serialize for Permission {
	fn serialize<S>(
		&self,
		serializer: S,
	) -> Result<S::Ok, S::Error>
	where
		S: Serializer,
	{
		match self {
			Permission::Info => serializer.serialize_str("info"),
			Permission::BoardsList => serializer.serialize_str("boards.list"),
			Permission::BoardsGet => serializer.serialize_str("boards.get"),
			Permission::BoardsPost => serializer.serialize_str("boards.post"),
			Permission::BoardsPatch => serializer.serialize_str("boards.patch"),
			Permission::BoardsDelete => serializer.serialize_str("boards.delete"),
			Permission::BoardsDataGet => serializer.serialize_str("boards.data.get"),
			Permission::BoardsDataPatch => serializer.serialize_str("boards.data.patch"),
			Permission::BoardsUsers => serializer.serialize_str("boards.users"),
			Permission::BoardsPixelsList => serializer.serialize_str("boards.pixels.list"),
			Permission::BoardsPixelsGet => serializer.serialize_str("boards.pixels.get"),
			Permission::BoardsPixelsPost => serializer.serialize_str("boards.pixels.post"),
			Permission::SocketCore => serializer.serialize_str("socket.core"),
			Permission::SocketAuthentication => serializer.serialize_str("socket.authentication"),
		}
	}
}

#[derive(Debug)]
pub enum PermissionsError {
	MissingPermission(Permission),
}

impl Reject for PermissionsError {}

pub fn with_permission(
	permission: Permission
) -> (impl Fn(AuthedUser) -> future::Ready<Result<AuthedUser, Rejection>> + Clone) {
	move |authed| {
		let has_perm = match Option::<&User>::from(&authed) {
			Some(user) => user.permissions.contains(&permission),
			None => {
				User::default()
					.permissions
					.contains(&permission)
			},
		};

		if has_perm {
			future::ok(authed)
		} else {
			future::err(warp::reject::custom(PermissionsError::MissingPermission(
				permission,
			)))
		}
	}
}
