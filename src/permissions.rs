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
	UsersPatch,
	UsersDelete,
	UsersCurrentGet,
	UsersCurrentPatch,
	UsersCurrentDelete,
	UsersRolesGet,
	UsersCurrentRolesGet,
	RolesList,
	RolesGet,
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
			Permission::UsersPatch => "users.patch",
			Permission::UsersDelete => "users.delete",
			Permission::UsersCurrentGet => "users.current.get",
			Permission::UsersCurrentPatch => "users.current.patch",
			Permission::UsersCurrentDelete => "users.current.delete",
			Permission::UsersRolesGet => "users.roles.get",
			Permission::UsersCurrentRolesGet => "users.current.roles.get",
			Permission::RolesList => "roles.list",
			Permission::RolesGet => "roles.get",
			Permission::SocketCore => "socket.core",
			Permission::SocketAuthentication => "socket.authentication",
			Permission::SocketBoardsInitial => "socket.boards.initial",
			Permission::SocketBoardsMask => "socket.boards.mask",
			Permission::SocketBoardsTimestamps => "socket.boards.timestamps",
			Permission::SocketBoardLifecycle => "socket.boards.lifecycle",
		}
	}
}

impl TryFrom<&str> for Permission {
	type Error = ();

	fn try_from(value: &str) -> Result<Self, Self::Error> {
		// TODO: find an way to exhaustively match this
		// (might need a proc macro with custom FromStr rules)
		match value {
			"info" => Ok(Permission::Info),
			"boards.list" => Ok(Permission::BoardsList),
			"boards.get" => Ok(Permission::BoardsGet),
			"boards.post" => Ok(Permission::BoardsPost),
			"boards.patch" => Ok(Permission::BoardsPatch),
			"boards.delete" => Ok(Permission::BoardsDelete),
			"boards.data.get" => Ok(Permission::BoardsDataGet),
			"boards.data.patch" => Ok(Permission::BoardsDataPatch),
			"boards.users" => Ok(Permission::BoardsUsers),
			"boards.pixels.list" => Ok(Permission::BoardsPixelsList),
			"boards.pixels.get" => Ok(Permission::BoardsPixelsGet),
			"boards.pixels.post" => Ok(Permission::BoardsPixelsPost),
			"users.list" => Ok(Permission::UsersList),
			"users.get" => Ok(Permission::UsersGet),
			"users.patch" => Ok(Permission::UsersPatch),
			"users.delete" => Ok(Permission::UsersDelete),
			"users.current.get" => Ok(Permission::UsersCurrentGet),
			"users.current.patch" => Ok(Permission::UsersCurrentPatch),
			"users.current.delete" => Ok(Permission::UsersCurrentDelete),
			"users.roles.get" => Ok(Permission::UsersRolesGet),
			"users.current.roles.get" => Ok(Permission::UsersCurrentRolesGet),
			"roles.list" => Ok(Permission::UsersList),
			"roles.get" => Ok(Permission::RolesGet),
			"socket.core" => Ok(Permission::SocketCore),
			"socket.authentication" => Ok(Permission::SocketAuthentication),
			"socket.boards.initial" => Ok(Permission::SocketBoardsInitial),
			"socket.boards.mask" => Ok(Permission::SocketBoardsMask),
			"socket.boards.timestamps" => Ok(Permission::SocketBoardsTimestamps),
			"socket.boards.lifecycle" => Ok(Permission::SocketBoardLifecycle),
			_ => Err(()),
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
