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
	SocketBoardsInitial,
	SocketBoardsMask,
	SocketBoardsTimestamps,
}

impl From<&Permission> for &str {
	fn from(permission: &Permission) -> Self {
		match permission {
			Permission::Info => "info",
			Permission::BoardsList => "boards.list",
			Permission::BoardsGet => "boards.get",
			Permission::BoardsPost => "boards.post",
			Permission::BoardsPatch => "boards.patch",
			Permission::BoardsDelete => "boards.delete",
			Permission::BoardsDataGet => "boards.data.get",
			Permission::BoardsDataPatch => "boards.data.patch",
			Permission::BoardsUsers => "boards.users",
			Permission::BoardsPixelsList => "boards.pixels.list",
			Permission::BoardsPixelsGet => "boards.pixels.get",
			Permission::BoardsPixelsPost => "boards.pixels.post",
			Permission::SocketCore => "socket.core",
			Permission::SocketAuthentication => "socket.authentication",
			Permission::SocketBoardsInitial => "socket.boards.initial",
			Permission::SocketBoardsMask => "socket.boards.mask",
			Permission::SocketBoardsTimestamps => "socket.boards.timestamps",
		}
	}
}

impl Serialize for Permission {
	fn serialize<S: Serializer>(
		&self,
		serializer: S,
	) -> Result<S::Ok, S::Error> {
		serializer.serialize_str(self.into())
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
		let user = Option::<&User>::from(&authed).unwrap_or_default();

		if user.permissions.contains(&permission) {
			future::ok(authed)
		} else {
			let error =PermissionsError::MissingPermission(permission);
			future::err(warp::reject::custom(error))
		}
	}
}
