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
	fn serialize<S: Serializer>(
		&self,
		serializer: S,
	) -> Result<S::Ok, S::Error> {
		let permission_str = match self {
			Self::Info => "info",
			Self::BoardsList => "boards.list",
			Self::BoardsGet => "boards.get",
			Self::BoardsPost => "boards.post",
			Self::BoardsPatch => "boards.patch",
			Self::BoardsDelete => "boards.delete",
			Self::BoardsDataGet => "boards.data.get",
			Self::BoardsDataPatch => "boards.data.patch",
			Self::BoardsUsers => "boards.users",
			Self::BoardsPixelsList => "boards.pixels.list",
			Self::BoardsPixelsGet => "boards.pixels.get",
			Self::BoardsPixelsPost => "boards.pixels.post",
			Self::SocketCore => "socket.core",
			Self::SocketAuthentication => "socket.authentication",
		};

		serializer.serialize_str(permission_str)
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
		let user = Option::<&User>::from(&authed)
			.unwrap_or_default();

		if user.permissions.contains(&permission) {
			future::ok(authed)
		} else {
			future::err(warp::reject::custom(PermissionsError::MissingPermission(
				permission,
			)))
		}
	}
}
