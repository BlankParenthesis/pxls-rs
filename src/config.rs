use serde::Deserialize;
use url::Url;

#[derive(Deserialize)]
pub struct Config {
	pub host: String,
	pub port: u16,
	pub database_url: Url,
	pub oidc_issuer: Url,
	pub oidc_client_id: Option<String>,
	pub ldap_url: Url,
	pub ldap_manager_user: String,
	pub ldap_manager_password: String,
	pub ldap_base: String,
	// TODO: defaults for ldap stuff
	pub ldap_users_ou: String,
	pub ldap_users_id_field: String,
	pub ldap_users_username_field: String,
	pub ldap_roles_ou: String,
	pub ldap_factions_ou: String,
	pub undo_deadline_seconds: u32,
}

impl Config {
	pub fn discovery_url(&self) -> Url {
		self.oidc_issuer.join(".well-known/openid-configuration").unwrap()
	}
}

lazy_static! {
	pub static ref CONFIG: Config = envy::from_env::<Config>()
		.expect("Incomplete config setup");
}

pub fn check() {
	if CONFIG.host.is_empty() {
		panic!("Missing HOST value");
	}

	if CONFIG.ldap_base.is_empty() {
		panic!("Missing LDAP_BASE value");
	}

	if CONFIG.ldap_users_ou.is_empty() {
		panic!("Missing LDAP_USERS_OU value");
	}

	if CONFIG.ldap_users_id_field.is_empty() {
		panic!("Missing LDAP_USERS_ID_FIELD value");
	}

	if CONFIG.ldap_users_username_field.is_empty() {
		panic!("Missing LDAP_USERS_USERNAME_FIELD value");
	}

	if CONFIG.ldap_roles_ou.is_empty() {
		panic!("Missing LDAP_ROLES_OU value");
	}

	if CONFIG.ldap_factions_ou.is_empty() {
		panic!("Missing LDAP_FACTIONS_OU value");
	}
}
