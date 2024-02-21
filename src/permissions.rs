use std::fmt;

use enumset::{EnumSet, EnumSetType};
use serde::{Serialize, Serializer, Deserialize, de::Visitor};

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
	UsersRolesPost,
	UsersRolesDelete,
	UsersCurrentRolesGet,
	UsersCurrentRolesPost,
	UsersCurrentRolesDelete,
	RolesList,
	RolesGet,
	RolesPost,
	RolesPatch,
	RolesDelete,
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

	pub fn to_current(&self) -> Option<Self> {
		match self {
			Self::UsersGet => Some(Self::UsersCurrentGet),
			Self::UsersPatch => Some(Self::UsersCurrentPatch),
			Self::UsersDelete => Some(Self::UsersCurrentDelete),
			Self::UsersRolesGet => Some(Self::UsersCurrentRolesGet),
			Self::UsersRolesPost => Some(Self::UsersCurrentRolesPost),
			Self::UsersRolesDelete => Some(Self::UsersCurrentRolesDelete),
			_ => None,
		}
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
			Permission::UsersRolesPost => "users.roles.post",
			Permission::UsersRolesDelete => "users.roles.delete",
			Permission::UsersCurrentRolesGet => "users.current.roles.get",
			Permission::UsersCurrentRolesPost => "users.current.roles.post",
			Permission::UsersCurrentRolesDelete => "users.current.roles.delete",
			Permission::RolesList => "roles.list",
			Permission::RolesGet => "roles.get",
			Permission::RolesPost => "roles.post",
			Permission::RolesPatch => "roles.patch",
			Permission::RolesDelete => "roles.delete",
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
			"users.roles.post" => Ok(Permission::UsersRolesPost),
			"users.roles.delete" => Ok(Permission::UsersRolesDelete),
			"users.current.roles.get" => Ok(Permission::UsersCurrentRolesGet),
			"users.current.roles.post" => Ok(Permission::UsersCurrentRolesPost),
			"users.current.roles.delete" => Ok(Permission::UsersCurrentRolesDelete),
			"roles.list" => Ok(Permission::UsersList),
			"roles.get" => Ok(Permission::RolesGet),
			"roles.post" => Ok(Permission::RolesPost),
			"roles.patch" => Ok(Permission::RolesPatch),
			"roles.delete" => Ok(Permission::RolesDelete),
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


struct V {}

impl<'de> Visitor<'de> for V {
	type Value = Permission;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("A permission string")
    }

	fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
	where E: serde::de::Error {
		Permission::try_from(v)
			.map_err(|e| E::custom("Invalid permission"))
	}
}

impl<'de> Deserialize<'de> for Permission {
    fn deserialize<D>(
		deserializer: D,
	) -> Result<Self, D::Error>
    where D: serde::Deserializer<'de> {
        deserializer.deserialize_str(V {})
    }
}
