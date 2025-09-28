use url::Url;

#[derive(Debug, Clone)]
pub struct User {
	pub id: String,
	pub name: String,
	pub created_at: i64,
}

#[derive(Debug, Clone)]
pub struct Role {
	pub name: String,
	pub icon: Option<Url>,
	pub permissions: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct RoleMembers {
	pub role: String,
	pub users: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Faction {
	pub cn: String,
	pub name: String,
	pub icon: Option<Url>,
	pub created_at: i64,
}

#[derive(Debug, Clone)]
pub struct FactionMember {
	pub user: String,
	pub owner: bool,
}

#[derive(Debug, Clone)]
pub struct FactionMembers {
	pub faction: String,
	pub users: Vec<FactionMember>,
}
