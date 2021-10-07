use std::collections::HashSet;
use std::future::Future;
use std::pin::Pin;
use actix_web::{FromRequest, HttpRequest, dev::Payload};
use actix_web_httpauth::extractors::{AuthenticationError, bearer::BearerAuth};
use actix_web_httpauth::headers::www_authenticate::bearer::Bearer;
use serde::Deserialize;
use http::StatusCode;

use crate::access::permissions::Permission;

pub struct User {
	pub id: Option<String>,
	pub permissions: HashSet<Permission>,
}

impl Default for User {
	fn default() -> Self {
		let mut permissions = HashSet::new();
		permissions.insert(Permission::Info);
		permissions.insert(Permission::BoardsList);
		permissions.insert(Permission::BoardsGet);
		permissions.insert(Permission::BoardsPost);
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

#[derive(Deserialize)]
struct UserInfo {
	sub: String,
}

impl FromRequest for User {
	type Error = actix_web::Error;
	type Future = Pin<Box<dyn Future<Output = Result<Self, Self::Error>>>>;
	type Config = ();
	
	fn from_request(req: &HttpRequest, payload: &mut Payload) -> Self::Future {
		let auth = BearerAuth::from_request(req, payload).into_inner();
		Box::pin(async {
			match auth {
				Ok(auth) => {
					let client = actix_web::client::Client::new();

					let config = crate::config::CONFIG.read().unwrap();
					let userinfo_endpoint = 
						config.oidc_configration(&client).await?
						.userinfo_endpoint.ok_or_else(|| 
							actix_web::error::ErrorBadGateway(
								"Identity provider lacks userinfo endpoint"
							)
						)?;

					let mut response = client
						.get(userinfo_endpoint)
						.header("Authorization", format!("Bearer {}", auth.token()))
						.send().await
						.map_err(actix_web::error::ErrorBadGateway)?;

					match response.status() {
						StatusCode::OK => {
							let mut permissions = HashSet::new();
							// TODO: user permissions
							permissions.insert(Permission::BoardsPixelsPost);
							permissions.insert(Permission::BoardsGet);
							response.json().await
								.map(|user_info: UserInfo| User {
									id: Some(user_info.sub),
									permissions,
								})
								.map_err(actix_web::error::ErrorBadGateway)
						},
						StatusCode::UNAUTHORIZED => {
							Err(Self::Error::from(AuthenticationError::new(
								Bearer::build()
									.finish()
							)))
						},
						code => Err(actix_web::error::ErrorBadGateway(
							format!("Got unexpected response from identity provider: {}", code))
						)
					}
				},
				Err(e) => Err(e.into()),
			}
		})
	}
}
