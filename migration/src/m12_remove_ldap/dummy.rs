use super::entities::{User, Role, RoleMembers, Faction, FactionMembers};

#[derive(Debug)]
pub enum Error {}

pub struct Connection;

impl Connection {
	pub async fn new() -> Result<Self, Error> { Ok(Self {}) }
	pub async fn load_users(&mut self) -> Result<Vec<User>, Error> { Ok(vec![] )}
	pub async fn load_roles(&mut self) -> Result<Vec<Role>, Error> { Ok(vec![])}
	pub async fn load_role_members(&mut self) -> Result<Vec<RoleMembers>, Error> { Ok(vec![])}
	pub async fn load_factions(&mut self) -> Result<Vec<Faction>, Error> { Ok(vec![])}
	pub async fn load_faction_members(&mut self) -> Result<Vec<FactionMembers>, Error> { Ok(vec![])}
}
