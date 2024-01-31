use std::{
	collections::HashSet,
	hash::{Hash, Hasher},
	time::{Duration, SystemTime, UNIX_EPOCH},
};

use jsonwebtoken::TokenData;

use crate::{access::permissions::Permission, authentication::openid::Identity};

#[derive(Debug, Clone, Eq)]
pub struct User {
	pub id: Option<String>,
	pub permissions: HashSet<Permission>,
}

impl User {
	pub fn from_id(id: String) -> Self {
		let mut permissions = User::default().permissions;

		// TODO: permissions
		permissions.insert(Permission::BoardsPixelsPost);

		Self {
			id: Some(id),
			permissions,
		}
	}
}

lazy_static! {
	static ref DEFAULT_USER: User = User::default();
}

impl Default for &User {
	fn default() -> Self {
		&DEFAULT_USER
	}
}

impl Default for User {
	fn default() -> Self {
		let permissions = HashSet::from([
			Permission::Info,
			Permission::BoardsList,
			Permission::BoardsGet,
			Permission::BoardsPost,
			Permission::BoardsPatch,
			Permission::BoardsDelete,
			Permission::BoardsDataGet,
			Permission::BoardsUsers,
			Permission::BoardsPixelsList,
			Permission::BoardsPixelsGet,
			Permission::SocketCore,
			Permission::SocketAuthentication,
			Permission::SocketBoardsInitial,
			Permission::SocketBoardsMask,
			Permission::SocketBoardsTimestamps,
		]);

		Self {
			id: None,
			permissions,
		}
	}
}

impl PartialEq for User {
	fn eq(
		&self,
		other: &Self,
	) -> bool {
		self.id == other.id
	}
}

impl Hash for User {
	fn hash<H: Hasher>(
		&self,
		state: &mut H,
	) {
		self.id.hash(state);
	}
}

#[derive(Debug, Clone, Eq)]
pub enum AuthedUser {
	Authed { user: User, valid_until: SystemTime },
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
			AuthedUser::Authed { user, .. } => Some(user),
			AuthedUser::None => None,
		}
	}
}

impl<'l> From<&'l AuthedUser> for Option<&'l User> {
	fn from(authed: &'l AuthedUser) -> Self {
		match authed {
			AuthedUser::Authed { ref user, .. } => Some(user),
			AuthedUser::None => None,
		}
	}
}

impl From<TokenData<Identity>> for AuthedUser {
	fn from(token_data: TokenData<Identity>) -> Self {
		Self::Authed {
			valid_until: UNIX_EPOCH + Duration::from_secs(token_data.claims.exp),
			user: User::from(token_data.claims),
		}
	}
}

impl PartialEq for AuthedUser {
	fn eq(
		&self,
		other: &Self,
	) -> bool {
		match (self, other) {
			(
				Self::Authed { user: l_user, .. },
				Self::Authed { user: r_user, .. },
			) => l_user == r_user,
			(Self::None, Self::None) => true,
			_ => false,
		}
	}
}

impl Hash for AuthedUser {
	fn hash<H: Hasher>(
		&self,
		state: &mut H,
	) {
		Option::<&User>::from(self).hash(state);
	}
}

impl AuthedUser {
	pub fn user(&self) -> Option<&User> {
		Option::<&User>::from(self)
	}
}
