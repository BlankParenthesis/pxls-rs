use std::collections::HashSet;

use http::StatusCode;
use url::Url;
use serde::Deserialize;

use actix_web::client::Client;
use actix_web::error::Error;

use jsonwebkey::JsonWebKey;

use jsonwebtoken::{decode, decode_header, Validation, TokenData};
use jsonwebtoken::errors::{Error as JWTError};

use crate::access::permissions::Permission;
use crate::objects::User;

#[derive(Deserialize)]
pub struct Discovery {
	pub issuer: Url,
	pub jwks_uri: Url,
}

impl Discovery {
	// TODO: cache
	pub async fn load(
		discovery_url: Url,
		client: &Client,
	) -> Result<Self, Error> {
		let mut response = client
			.get(discovery_url.to_string())
			.send().await?;
		
		match response.status() {
			StatusCode::OK => {
				response.json().await
					.map_err(|_| actix_web::error::ErrorBadGateway(
						"identity provider gave invalid Open ID configuration"
					))
			},
			code => Err(actix_web::error::ErrorBadGateway(
				format!("Got unexpected response from identity provider: {}", code))
			)
		}
	}

	// TODO: cache
	pub async fn jwks_keys(
		&self,
		client: &Client,
	) -> Result<Vec<JsonWebKey>, Error> {
		let mut response = client
			.get(self.jwks_uri.to_string())
			.send().await?;

		match response.status() {
			StatusCode::OK => {
				#[derive(Deserialize)]
				struct Keys {
					keys: Vec<JsonWebKey>,
				}

				response.json::<Keys>().await
					.map(|json| json.keys)
					.map_err(|_| actix_web::error::ErrorBadGateway(
						"identity provider gave invalid Open ID configuration"
					))
			},
			code => Err(actix_web::error::ErrorBadGateway(
				format!("Got unexpected response from identity provider: {}", code))
			)
		}
	}
}

#[derive(Deserialize)]
pub struct Identity {
	sub: String,
}

impl From<Identity> for User {
    fn from(identity: Identity) -> Self {
		let mut permissions = HashSet::new();

		// TODO: permissions properly
		permissions.insert(Permission::BoardsPixelsPost);
		permissions.insert(Permission::BoardsGet);
		permissions.insert(Permission::SocketCore);

		User {
			id: Some(identity.sub),
			permissions,
		}
    }
}


#[derive(Debug)]
pub enum ValidationError {
	JWTError(JWTError),
	DiscoveryError(Error),
	NoValidKeys,
}

impl From<JWTError> for ValidationError {
	fn from(jwt_error: JWTError) -> Self {
		Self::JWTError(jwt_error)
	}
}

impl From<Error> for ValidationError {
	fn from(actix_error: Error) -> Self {
		Self::DiscoveryError(actix_error)
	}
}

pub async fn validate_token(token: &str) -> Result<TokenData<Identity>, ValidationError> {
	let client = Client::new();
	let discovery_url = crate::config::CONFIG.read().unwrap().discovery_url();
	
	let discovery = Discovery::load(discovery_url, &client).await?;
	let keys = discovery.jwks_keys(&client).await?;
	
	let header = decode_header(token)?;

	let matching_key = if header.kid.is_some() {
		keys.iter()
			.filter(|key| key.algorithm.is_some())
			.find(|key| key.key_id.as_deref() == header.kid.as_deref())
	} else {
		keys.iter()
			.filter(|key| key.algorithm.is_some())
			.find(|key| header.alg == key.algorithm.unwrap().into())
	};

	if let Some(key) = matching_key {
		decode::<Identity>(
			token,
			&key.key.to_decoding_key(),
			&Validation::new(key.algorithm.unwrap().into())
		).map_err(ValidationError::from)
	} else {
		Err(ValidationError::NoValidKeys)
	}
}