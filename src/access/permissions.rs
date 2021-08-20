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

// creates a named guard which succeeds if the client has all specified permissions
macro_rules! guard {
	( $guard_name:ident, $( $permission:ident ),* ) => {
		pub struct $guard_name {}

		impl actix_web::FromRequest for $guard_name {
			type Config = ();
			type Error = actix_web::error::Error;
			type Future = futures_util::future::Ready<Result<Self, Self::Error>>;

			/// Convert request to a Self
			fn from_request(_request: &actix_web::HttpRequest, _payload: &mut actix_web::dev::Payload) -> Self::Future {
				let mut required_permissions = std::collections::HashSet::new();
				$(
					required_permissions.insert(crate::access::permissions::Permission::$permission);
				)*

				if crate::access::permissions::DEFAULT_PERMISSIONS.is_superset(&required_permissions) {
					futures_util::future::ok($guard_name {})
				} else {
					futures_util::future::err(actix_web::error::ErrorForbidden("Missing Permissions"))
				}
			}
		}
	}
}