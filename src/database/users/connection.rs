use async_trait::async_trait;

use ldap3::{LdapConnAsync, LdapError, drive, Ldap};
use deadpool::managed::{Manager, Metrics, RecycleResult};

pub type Pool = deadpool::managed::Pool<LDAPConnectionManager>;
pub type Connection = deadpool::managed::Object<LDAPConnectionManager>;


pub struct LDAPConnectionManager(pub String);

#[async_trait]
impl Manager for LDAPConnectionManager {
	type Type = Ldap;
	type Error = LdapError;

	async fn create(&self) -> Result<Self::Type, Self::Error> {
		let (connection, ldap) = LdapConnAsync::new(&self.0).await?;
		drive!(connection);
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
