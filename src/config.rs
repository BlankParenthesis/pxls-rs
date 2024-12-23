use serde::Deserialize;
use url::Url;

fn default_ldap_users_ou() -> String {
	"users".to_string()
}

fn default_users_id_field() -> String {
	"uid".to_string()
}

fn default_users_username_field() -> String {
	"displayName".to_string()
}

fn default_roles_ou() -> String {
	"roles".to_string()
}

fn default_factions_ou() -> String {
	"factions".to_string()
}

fn default_cooldown() -> u32 {
	30
}

fn default_board() -> usize {
	1
}

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
	#[serde(default = "default_ldap_users_ou")]
	pub ldap_users_ou: String,
	#[serde(default = "default_users_id_field")]
	pub ldap_users_id_field: String,
	#[serde(default = "default_users_username_field")]
	pub ldap_users_username_field: String,
	#[serde(default = "default_roles_ou")]
	pub ldap_roles_ou: String,
	#[serde(default = "default_factions_ou")]
	pub ldap_factions_ou: String,
	pub undo_deadline_seconds: u32,
	/// The role granted to all authenticated users.
	pub default_role: Option<String>,
	/// The role granted to all unauthentited users.
	/// If None, default_role will be used.
	pub unauthenticated_role: Option<String>,
	// If True, self-signed certificate responses from the oidc server will be accepted
	#[serde(default)]
	pub allow_oidc_selfsigned: bool,
	// Cooldown in seconds
	#[serde(default = "default_cooldown")]
	pub cooldown: u32,
	// Default board ID
	#[serde(default = "default_board")]
	pub default_board: usize,
	// The maximum number of times per second to send place packets.
	// Setting a lower value improves network efficiency at the cost of
	// increased client visual latency.
	#[serde(default)]
	pub network_tickrate: Option<f32>,
	// The maximum number of times per second to sync placements to the database.
	// Setting a lower value reduces disk writes at the cost of a higher data
	// loss risk on unexpected shutdown.
	#[serde(default)]
	pub database_tickrate: Option<f32>,
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

	if CONFIG.default_role.as_ref().map(|s| s.is_empty()).unwrap_or(false) {
		eprintln!("Warning: default user role is the empty string");
	}

	if CONFIG.unauthenticated_role.as_ref().map(|s| s.is_empty()).unwrap_or(false) {
		eprintln!("Warning: unauthenticated user role is the empty string");
	}

	if CONFIG.network_tickrate.map(|r| r <= 0.0).unwrap_or(false) {
		panic!("Network tickrate must be greater than 0");
	}

	if CONFIG.database_tickrate.map(|r| r <= 0.0).unwrap_or(false) {
		panic!("database tickrate must be greater than 0");
	}
}
