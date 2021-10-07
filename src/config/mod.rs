use serde::Deserialize;
use url::Url;
use std::sync::RwLock;

use actix_web::{error, client::Client};
use http::StatusCode;

#[derive(Deserialize)]
pub struct Config {
	pub host: String,
	pub port: u16,
	pub database_url: Url,
	pub oidc_issuer: Url,
	pub oidc_client_id: Option<String>,
}

#[derive(Deserialize)]
pub struct OpenIdConfiguration {
	pub userinfo_endpoint: Option<String>,
}

impl Config {
	// TODO: cache response
	pub async fn oidc_configration(&self, client: &Client) -> Result<OpenIdConfiguration, error::Error> {
		let mut response = client
			.get(self.oidc_issuer
				.join(".well-known/openid-configuration").unwrap()
				.to_string())
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
}


lazy_static! {
	pub static ref CONFIG: RwLock<Config> = 
		RwLock::new(envy::from_env::<Config>()
			.expect("Incomplete config setup"));
}
