use std::collections::HashSet;

use crate::access::permissions::Permission;

#[derive(Debug, Clone)]
pub struct User {
	pub id: Option<String>,
	pub permissions: HashSet<Permission>,
}

impl User {
	pub fn from_id(id: String) -> Self {
		let mut permissions = HashSet::new();

		// TODO: permissions
		permissions.insert(Permission::BoardsPixelsPost);
		permissions.insert(Permission::BoardsGet);
		permissions.insert(Permission::SocketCore);

		Self {
			id: Some(id),
			permissions,
		}
	}
}

impl Default for User {
	fn default() -> Self {
		let mut permissions = HashSet::new();
		permissions.insert(Permission::Info);
		permissions.insert(Permission::BoardsList);
		permissions.insert(Permission::BoardsGet);
		//permissions.insert(Permission::BoardsPost);
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

impl From<AuthedUser> for User {
	fn from(authed: AuthedUser) -> Self {
		Option::<Self>::from(authed).unwrap_or_default()
	}
}

impl From<AuthedUser> for Option<User> {
	fn from(authed: AuthedUser) -> Self {
		match authed {
			AuthedUser::Authed(user) => Some(user),
			AuthedUser::None => None,
		}
	}
}

impl<'l> From<&'l AuthedUser> for Option<&'l User> {
	fn from(authed: &'l AuthedUser) -> Self {
		match authed {
			AuthedUser::Authed(ref user) => Some(user),
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

