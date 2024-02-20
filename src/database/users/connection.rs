use async_trait::async_trait;

use ldap3::{LdapConnAsync, LdapError, drive, Ldap};
use deadpool::managed::{Manager, Metrics, RecycleResult};

use crate::config::CONFIG;

pub type Pool = deadpool::managed::Pool<LDAPConnectionManager>;
pub type Connection = deadpool::managed::Object<LDAPConnectionManager>;


pub struct LDAPConnectionManager(pub String);

#[async_trait]
impl Manager for LDAPConnectionManager {
	type Type = Ldap;
	type Error = LdapError;

	async fn create(&self) -> Result<Self::Type, Self::Error> {
		let (connection, mut ldap) = LdapConnAsync::new(&self.0).await?;
		drive!(connection);
		let user = format!(
			"cn={},{}",
			CONFIG.ldap_manager_user.as_str(),
			CONFIG.ldap_base,
		);
		let password = CONFIG.ldap_manager_password.as_str();
		ldap.simple_bind(&user, password).await?.success()?;
		Ok(ldap)
	}

	async fn recycle(
		&self,
		_connection: &mut Self::Type,
		_metrics: &Metrics,
	) -> RecycleResult<Self::Error> {
		// TODO: maybe the connection should be checked for errors?
		// the r2d2 crate runs a whoami query to check this, but doing that
		// for every connection pulled out of the pool seems very wasteful.
		Ok(())
	}
}
