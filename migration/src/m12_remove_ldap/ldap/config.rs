use url::Url;
use serde::Deserialize;
use lazy_static::lazy_static;

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

#[derive(Deserialize)]
pub struct Config {
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
}

lazy_static! {
	pub static ref CONFIG: Config = envy::from_env::<Config>()
		.expect("Incomplete config setup");
}
