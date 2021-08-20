use rocket::http;
use std::fmt;
use std::collections::HashSet;
use lazy_static;
use serde::{Serialize, Serializer};

#[derive(PartialEq, Eq, Hash)]
pub enum Permission {
	Info,
	BoardsList,
	BoardsGet,
	SocketCore,
}

impl Serialize for Permission {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where S: Serializer {
        match self {
			Permission::Info => serializer.serialize_str("info"),
			Permission::BoardsList => serializer.serialize_str("boards.list"),
			Permission::BoardsGet => serializer.serialize_str("boards.get"),
			Permission::SocketCore => serializer.serialize_str("socket.core"),
		}
    }
}

lazy_static! {
	pub static ref DEFAULT_PERMISSIONS: HashSet<Permission> = {
		let mut set = HashSet::new();
		set.insert(Permission::Info);
		set.insert(Permission::BoardsList);
		set.insert(Permission::BoardsGet);
		set.insert(Permission::SocketCore);
		set
	};
}

#[derive(std::fmt::Debug, Default)]
pub struct Error {}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Permissions Error")
    }
}

impl std::error::Error for Error {
}

impl From<Error> for (http::Status, Error) {
	fn from(error: Error) -> (http::Status, Error) {
		(http::Status { code: 403 }, error)
	}
}

// creates a named guard which succeeds if the client has all specified permissions
macro_rules! guard {
	( $guard_name:ident, $( $permission:ident ),* ) => {
		pub struct $guard_name {}

		#[async_trait]
		impl<'r> rocket::request::FromRequest<'r> for $guard_name {

			type Error = permissions::Error;

			async fn from_request(_request: &'r rocket::request::Request<'_>) -> rocket::request::Outcome<Self, Self::Error> {
				let mut required_permissions = std::collections::HashSet::new();
				$(
					required_permissions.insert(crate::access::permissions::Permission::$permission);
				)*

				if permissions::DEFAULT_PERMISSIONS.is_superset(&required_permissions) {
					rocket::request::Outcome::Success($guard_name {})
				} else {
					rocket::request::Outcome::Failure(permissions::Error::default().into())
				}
			}
		}
	}
}