use http::StatusCode;
use jsonwebkey::JsonWebKey;
use jsonwebtoken::{decode, decode_header, errors::Error as JWTError, TokenData, Validation};
use reqwest::Client;
use serde::Deserialize;
use url::Url;

use crate::objects::User;
use crate::config::CONFIG;

#[derive(Debug)]
pub enum DiscoveryError {
	ContactFailed,
	InvalidResponse,
	InvalidConfigResponse,
}

impl From<DiscoveryError> for StatusCode {
	fn from(error: DiscoveryError) -> Self {
		match error {
			DiscoveryError::InvalidResponse => StatusCode::BAD_GATEWAY,
			DiscoveryError::InvalidConfigResponse => StatusCode::BAD_GATEWAY,
			DiscoveryError::ContactFailed => StatusCode::GATEWAY_TIMEOUT,
		}
	}
}

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
	) -> Result<Self, DiscoveryError> {
		let response = client
			.get(discovery_url.to_string())
			.send()
			.await
			.map_err(|_| DiscoveryError::ContactFailed)?;

		match response.status() {
			StatusCode::OK => {
				response
					.json()
					.await
					.map_err(|_| DiscoveryError::InvalidConfigResponse)
			},
			code => Err(DiscoveryError::InvalidResponse),
		}
	}

	// TODO: cache
	pub async fn jwks_keys(
		&self,
		client: &Client,
	) -> Result<Vec<JsonWebKey>, DiscoveryError> {
		let response = client
			.get(self.jwks_uri.to_string())
			.send()
			.await
			.map_err(|_| DiscoveryError::ContactFailed)?;

		match response.status() {
			StatusCode::OK => {
				#[derive(Deserialize)]
				struct Keys {
					keys: Vec<serde_json::Value>,
				}

				response
					.json::<Keys>()
					.await
					.map(|json| {
						json.keys
							.into_iter()
							.filter_map(|k| serde_json::from_value::<JsonWebKey>(k).ok())
							.collect()
					})
					.map_err(|e| {
						println!("{:?}", e);
						e
					})
					.map_err(|_| DiscoveryError::InvalidConfigResponse)
			},
			code => Err(DiscoveryError::InvalidResponse),
		}
	}
}

#[derive(Deserialize)]
pub struct Identity {
	pub sub: String,
	pub exp: u64,
}

impl From<Identity> for User {
	fn from(identity: Identity) -> Self {
		Self::from_id(identity.sub)
	}
}

#[derive(Debug)]
pub enum ValidationError {
	JWTError(JWTError),
	DiscoveryError(DiscoveryError),
	NoValidKeys,
}

impl From<JWTError> for ValidationError {
	fn from(jwt_error: JWTError) -> Self {
		Self::JWTError(jwt_error)
	}
}

impl From<DiscoveryError> for ValidationError {
	fn from(error: DiscoveryError) -> Self {
		Self::DiscoveryError(error)
	}
}

pub async fn validate_token(token: &str) -> Result<TokenData<Identity>, ValidationError> {
	let client = Client::new();
	let discovery_url = CONFIG.discovery_url();

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
			&Validation::new(key.algorithm.unwrap().into()),
		)
		.map_err(ValidationError::from)
	} else {
		Err(ValidationError::NoValidKeys)
	}
}
