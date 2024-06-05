use std::collections::HashMap;
use std::time::{SystemTime, Duration};

use tokio::sync::RwLock;
use warp::http::StatusCode;
use jsonwebkey::JsonWebKey;
use jsonwebtoken::{decode, decode_header, TokenData, Validation, Algorithm};
use jsonwebtoken::errors::Error as JWTError;
use reqwest::Client;
use serde::Deserialize;
use url::Url;

use crate::config::CONFIG;

#[derive(Debug, Clone)]
pub enum DiscoveryError {
	ContactFailed,
	InvalidResponse,
	InvalidConfigResponse,
}

impl From<DiscoveryError> for StatusCode {
	fn from(error: DiscoveryError) -> Self {
		match error {
			// TODO: these might be the wrong errors?
			// we're not really a proxy or a gateway (reverse proxy)
			DiscoveryError::InvalidResponse => StatusCode::BAD_GATEWAY,
			DiscoveryError::InvalidConfigResponse => StatusCode::BAD_GATEWAY,
			DiscoveryError::ContactFailed => StatusCode::GATEWAY_TIMEOUT,
		}
	}
}

#[derive(Deserialize)]
struct Discovery {
	#[allow(dead_code)]
	issuer: Url,
	jwks_uri: Url,
}

impl Discovery {
	pub async fn load(
		discovery_url: Url,
		client: &Client,
	) -> Result<Self, DiscoveryError> {
		let response = client
			.get(discovery_url.to_string())
			.send().await
			.map_err(|_| DiscoveryError::ContactFailed)?;

		match response.status() {
			StatusCode::OK => {
				response
					.json().await
					.map_err(|_| DiscoveryError::InvalidConfigResponse)
			},
			code => Err(DiscoveryError::InvalidResponse),
		}
	}

	pub async fn jwks_keys(
		&self,
		client: &Client,
	) -> Result<Vec<JsonWebKey>, DiscoveryError> {
		let response = client
			.get(self.jwks_uri.to_string())
			.send().await
			.map_err(|_| DiscoveryError::ContactFailed)?;

		match response.status() {
			StatusCode::OK => {
				#[derive(Deserialize)]
				struct Keys {
					keys: Vec<serde_json::Value>,
				}

				response
					.json::<Keys>().await
					.map(|json| {
						json.keys
							.into_iter()
							// TODO: this silently drops parsing errors on keys
							.filter_map(|k| serde_json::from_value(k).ok())
							.collect()
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

#[derive(Debug, Clone)]
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

fn find_key_by_id(
	id: String,
) -> impl FnMut(&JsonWebKey) -> bool {
	move |key| key.key_id.as_deref() == Some(id.as_str())
}

/// Safety: assumes key algorithm is not None
unsafe fn find_key_by_algorithm(
	algorithm: Algorithm,
) -> impl FnMut(&JsonWebKey) -> bool {
	move |key| {
		let key_algorithm = unsafe {
			key.algorithm.unwrap_unchecked()
		};
		algorithm == key_algorithm.into()
	}
}

lazy_static! {
	static ref CLIENT: Client = Client::new();
	static ref CACHED_DECODE_KEYS: RwLock<DecodeKeysCache> = {
		RwLock::new(DecodeKeysCache::default())
	};
	static ref CACHED_TOKENS: RwLock<HashMap<String, TokenData<Identity>>> = {
		RwLock::new(HashMap::new())
	};
}

#[derive(Debug)]
struct DecodeKeysCache {
	expiry: SystemTime,
	keys: Vec<JsonWebKey>,
}

impl Default for DecodeKeysCache {
	fn default() -> Self {
		// should immediately count as expired
		Self { expiry: SystemTime::now(), keys: vec![] }
	}
}

async fn get_decoding_keys() -> Result<Vec<JsonWebKey> , ValidationError> {
	let discovery_url = CONFIG.discovery_url();
	let cache = CACHED_DECODE_KEYS.read().await;
	let now = SystemTime::now();
	if cache.expiry <= now {
		let discovery = Discovery::load(discovery_url, &CLIENT).await?;
		let keys = discovery.jwks_keys(&CLIENT).await?;
		drop(cache);
		let mut cache = CACHED_DECODE_KEYS.write().await;
		cache.expiry = now + Duration::from_secs(5 * 60);
		cache.keys = keys.clone();
		Ok(keys)
	} else {
		Ok(cache.keys.clone())
	}
}

// TODO: consider using the crate "cached"
// FIXME: this cache is effectively a memory leak too since tokens expire and
// become new fairly often.
pub async fn validate_token(
	token: &str
) -> Result<TokenData<Identity>, ValidationError> {
	let token_string = token.to_string();
	let cache = CACHED_TOKENS.read().await;
	if let Some(cached) = cache.get(&token_string) {
		return Ok(clone_token(cached));
	}
	drop(cache);

	let decode_keys = get_decoding_keys().await?;

	let mut valid_keys = decode_keys
		.into_iter()
		.filter(|key| key.algorithm.is_some());

	let header = decode_header(token)?;

	let matching_key = if let Some(id) = header.kid {
		valid_keys.find(find_key_by_id(id))
	} else {
		// Safety: valid_keys only contains keys where algorithm is Some
		valid_keys.find(unsafe { find_key_by_algorithm(header.alg) })
	};

	let decode_key = matching_key.ok_or(ValidationError::NoValidKeys)?;

	// Safety: valid_keys only contains keys where algorithm is Some;
	// key came from matching_key which came from valid_keys.
	let algorithm = unsafe {
		decode_key.algorithm.unwrap_unchecked()
	};

	let token = decode::<Identity>(
		token,
		&decode_key.key.to_decoding_key(),
		&Validation::new(algorithm.into()),
	).map_err(ValidationError::from)?;
	let mut cache = CACHED_TOKENS.write().await;
	cache.insert(token_string, clone_token(&token));
	Ok(token)
}

// since token doesn't implement clone in the current version of jwt
fn clone_token(token: &TokenData<Identity>) -> TokenData<Identity> {
	let identity = Identity {
		sub: token.claims.sub.clone(),
		exp: token.claims.exp,
	};
	TokenData {
		header: token.header.clone(),
		claims: identity,
	}
}