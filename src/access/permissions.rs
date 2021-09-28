use serde::{Serialize, Serializer};

#[derive(PartialEq, Eq, Hash, Debug, Clone)]
pub enum Permission {
	Info,
	BoardsList,
	BoardsGet,
	BoardsPost,
	BoardsPatch,
	BoardsDelete,
	BoardsData,
	BoardsUsers,
	BoardsPixelsList,
	BoardsPixelsGet,
	BoardsPixelsPost,
	SocketCore,
}

impl Serialize for Permission {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where S: Serializer {
		match self {
			Permission::Info => serializer.serialize_str("info"),
			Permission::BoardsList => serializer.serialize_str("boards.list"),
			Permission::BoardsGet => serializer.serialize_str("boards.get"),
			Permission::BoardsPost => serializer.serialize_str("boards.post"),
			Permission::BoardsPatch => serializer.serialize_str("boards.patch"),
			Permission::BoardsDelete => serializer.serialize_str("boards.delete"),
			Permission::BoardsData => serializer.serialize_str("boards.data"),
			Permission::BoardsUsers => serializer.serialize_str("boards.users"),
			Permission::BoardsPixelsList => serializer.serialize_str("boards.pixels.list"),
			Permission::BoardsPixelsGet => serializer.serialize_str("boards.pixels.get"),
			Permission::BoardsPixelsPost => serializer.serialize_str("boards.pixels.post"),
			Permission::SocketCore => serializer.serialize_str("socket.core"),
		}
	}
}

// creates a named guard which succeeds if the client has all specified permissions
macro_rules! guard {
	( $guard_name:ident, $( $permission:ident ),* ) => {
		pub struct $guard_name {}

		impl actix_web::FromRequest for $guard_name {
			type Config = ();
			type Error = actix_web::error::Error;
			type Future = std::pin::Pin<Box<dyn std::future::Future<Output = Result<Self, Self::Error>>>>;

			fn from_request(
				request: &actix_web::HttpRequest,
				payload: &mut actix_web::dev::Payload,
			) -> Self::Future {
				let mut required_permissions = std::collections::HashSet::new();
				$(
					required_permissions.insert(crate::access::permissions::Permission::$permission);
				)*

				let user = crate::objects::User::from_request(request, payload);

				Box::pin(async move {
					if user.await.unwrap_or_default().permissions.is_superset(&required_permissions) {
						Ok($guard_name {})
					} else {
						Err(actix_web::error::ErrorForbidden("Missing Permissions"))
					}
				})
			}
		}
	}
}