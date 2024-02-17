use enumset::{EnumSet, EnumSetType};
use serde::{Serialize, Serializer};

#[derive(Debug, EnumSetType)]
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
	UsersList,
	UsersGet,
	SocketCore,
	SocketAuthentication,
	SocketBoardsInitial,
	SocketBoardsMask,
	SocketBoardsTimestamps,
	SocketBoardLifecycle,
}

impl Permission {
	pub fn defaults() -> EnumSet<Self> {
		// TODO: better defaults
		EnumSet::all() - Self::BoardsPixelsPost
	}
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
			Permission::UsersList => "users.list",
			Permission::UsersGet => "users.get",
			Permission::SocketCore => "socket.core",
			Permission::SocketAuthentication => "socket.authentication",
			Permission::SocketBoardsInitial => "socket.boards.initial",
			Permission::SocketBoardsMask => "socket.boards.mask",
			Permission::SocketBoardsTimestamps => "socket.boards.timestamps",
			Permission::SocketBoardLifecycle => "socket.boards.timestamps",
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
