use serde::Deserialize;
use url::Url;
use std::sync::RwLock;

#[derive(Deserialize)]
pub struct Config {
	pub host: String,
	pub port: u16,
	pub db_host: String,
	pub db_port: u16,
	pub oidc_issuer: Url,
	pub oidc_client_id: Option<String>,
}

lazy_static! {
	pub static ref CONFIG: RwLock<Config> = 
		RwLock::new(envy::from_env::<Config>()
			.expect("Incomplete config setup"));
}
