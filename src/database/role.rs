use std::fmt;

use enumset::EnumSet;
use sea_orm::RelationTrait;
use sea_orm::{ActiveValue::NotSet, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect, QueryTrait, Set, StreamTrait, TransactionTrait};
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;
use url::Url;
use warp::http::Uri;

use crate::config::CONFIG;
use crate::filter::response::paginated_list::{Page, PageToken};
use crate::filter::response::reference::{Referencable, Reference};
use crate::permissions::Permission;
use crate::routes::roles::roles::RoleFilter;

use super::entities::*;
use super::specifier::{SpecifierParser, Id, SpecfierParseError, Specifier, PathPart, specifier_path};
use super::user::UserSpecifier;
use super::{Connection, DbResult, DatabaseError};

#[derive(Debug, Default, Deserialize)]
pub struct UserRolesPageToken(pub u32);
impl PageToken for UserRolesPageToken {}
impl From<&role::Model> for UserRolesPageToken {
	fn from(value: &role::Model) -> Self {
		Self(value.id as _)
	}
}
impl fmt::Display for UserRolesPageToken {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{}", self.0)
	}
}

#[derive(Debug, Eq, PartialEq, Clone, Copy, Hash)]
pub struct UserRolesListSpecifier {
	user: i32
}

impl UserRolesListSpecifier {
	pub fn user(&self) -> UserSpecifier {
		UserSpecifier(self.user)
	}
}

impl Specifier for UserRolesListSpecifier {
	fn filter(&self) -> sea_query::SimpleExpr {
		role_member::Column::Member.eq(self.user)
	}
	
	fn from_ids(ids: &[&str]) -> Result<Self, SpecfierParseError> {
		let user = ids[0].parse()?;
		Ok(Self { user })
	}
	
	fn to_ids(&self) -> Box<[Id]> {
		Box::new([Id::I32(self.user)])
	}
	
	fn parts() -> &'static [PathPart] {
		specifier_path!("users", user, "roles")
	}
}

impl<'de> Deserialize<'de> for UserRolesListSpecifier {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where D: serde::Deserializer<'de> {
		deserializer.deserialize_str(SpecifierParser::new("A user roles list uri"))
	}
}

impl Serialize for UserRolesListSpecifier {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where S: serde::Serializer {
		serializer.serialize_str(self.to_uri().path())
	}
}

#[derive(Debug, Eq, PartialEq, Clone, Copy, Hash)]
pub struct RoleSpecifier(pub(super) i32);

impl Specifier for RoleSpecifier {
	fn filter(&self) -> sea_query::SimpleExpr {
		role::Column::Id.eq(self.0)
	}
	
	fn from_ids(ids: &[&str]) -> Result<Self, SpecfierParseError> {
		let role = ids[0].parse()?;
		Ok(Self(role))
	}
	
	fn to_ids(&self) -> Box<[Id]> {
		Box::new([Id::I32(self.0)])
	}
	
	fn parts() -> &'static [PathPart] {
		specifier_path!("roles", role)
	}
}

impl<'de> Deserialize<'de> for RoleSpecifier {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where D: serde::Deserializer<'de> {
		deserializer.deserialize_str(SpecifierParser::new("A role uri"))
	}
}

impl Serialize for RoleSpecifier {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where S: serde::Serializer {
		serializer.serialize_str(self.to_uri().path())
	}
}

#[skip_serializing_none]
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Role {
	#[serde(skip_serializing)]
	id: RoleSpecifier,
	pub name: String,
	pub icon: Option<Url>,
	pub permissions: Vec<Permission>,
}

impl Role {
	fn specifier(&self) -> &RoleSpecifier {
		&self.id
	}
}

impl Referencable for Role {
	fn uri(&self) -> Uri {
		self.id.to_uri()
	}
}

impl From<role::Model> for Role {
	fn from(role: role::Model) -> Self {
		let role::Model { id, name, icon, permissions } = role;
		let id = RoleSpecifier(id);
		// silently drops invalid icon urls
		let icon = icon.and_then(|icon| icon.parse().ok());
		// silently drops invalid permissions
		let permissions = permissions.split(',')
			.map(str::trim)
			.map(Permission::try_from)
			.filter_map(Result::ok)
			.collect::<EnumSet<_>>()
			.into_iter()
			.collect();
		Role { id, name, icon, permissions }
	}
}

#[derive(Debug, Default, Deserialize)]
pub struct RolesPageToken(pub u32);
impl PageToken for RolesPageToken {}
impl From<&role::Model> for RolesPageToken {
	fn from(value: &role::Model) -> Self {
		Self(value.id as _)
	}
}
impl fmt::Display for RolesPageToken {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{}", self.0)
	}
}

impl<C: TransactionTrait + ConnectionTrait + StreamTrait> Connection<C> {
	
	pub async fn list_roles(
		&self,
		page: RolesPageToken,
		limit: usize,
		filter: RoleFilter,
	) -> DbResult<Page<Reference<Role>>> {
		let roles = role::Entity::find()
			.filter(role::Column::Id.gt(page.0))
			.apply_if(filter.name, |q, name| q.filter(role::Column::Name.like(format!("%{name}%"))))
			.apply_if(filter.icon, |q, icon| q.filter(role::Column::Icon.like(format!("%{icon}%"))))
			.order_by(role::Column::Id, sea_orm::Order::Asc)
			.limit(Some((limit + 1) as u64))
			.all(&self.connection).await?;
		
		let next = roles.windows(2).nth(limit.saturating_sub(1))
			.map(|pair| &pair[0])
			.map(RolesPageToken::from)
			.map(|token| {
				// TODO: filter
				format!("/roles?page={token}&limit={limit}").parse().unwrap()
			});
		
		let items = roles.into_iter()
			.take(limit)
			.map(Role::from)
			.map(Reference::from)
			.collect();
		
		// TODO: previous
		Ok(Page { items, next, previous: None })
	}
	
	pub async fn get_role(
		&self,
		role: &RoleSpecifier,
	) -> DbResult<Option<Role>> {
		role::Entity::find()
			.filter(role::Column::Id.eq(role.0))
			.one(&self.connection).await
			.map(|r| r.map(Role::from))
			.map_err(DatabaseError::from)
	}
	
	pub async fn create_role(
		&self,
		name: String,
		icon: Option<Url>,
		permissions: EnumSet<Permission>,
	) -> DbResult<Reference<Role>> {
		let icon = icon.map(String::from);
		let permissions = permissions.iter()
			.map(|p| <&str>::from(&p))
			.map(String::from)
			.collect::<Vec<_>>()
			.join(",");
		
		let role = role::ActiveModel { 
			name: Set(name), 
			icon: Set(icon), 
			permissions: Set(permissions),
			..Default::default()
		};
		
		let insert = role::Entity::insert(role)
			.exec_with_returning(&self.connection).await?;
		
		Ok(Reference::from(Role::from(insert)))
	}
	
	pub async fn update_role(
		&self,
		role: &RoleSpecifier,
		name: Option<String>,
		icon: Option<Option<Url>>,
		permissions: Option<EnumSet<Permission>>,
	) -> DbResult<Option<Reference<Role>>> {
		let model = role::ActiveModel { 
			name: name.map(Set).unwrap_or(NotSet),
			icon: icon.map(|icon| Set(icon.map(String::from))).unwrap_or(NotSet),
			permissions: permissions.map(|p| {
				p.iter()
					.map(|p| <&str>::from(&p))
					.collect::<Vec<_>>()
					.join(",")
			}).map(Set).unwrap_or(NotSet),
			..Default::default()
		};
		
		let update = role::Entity::update_many()
			.set(model)
			.filter(role::Column::Id.eq(role.0))
			.exec_with_returning(&self.connection).await?;
		
		match update.as_slice() {
			[] => Ok(None),
			[user] => Ok(Some(Reference::from(Role::from(user.clone())))),
			_ => panic!("updated multiple roles with the same name"),
		}
	}
	
	pub async fn delete_role(
		&self,
		role: &RoleSpecifier,
	) -> DbResult<Option<()>> {
		let delete = role::Entity::delete_by_id(role.0)
			.exec(&self.connection).await?;
		
		match delete.rows_affected {
			0 => Ok(None),
			1 => Ok(Some(())),
			_ => panic!("deleted multiple roles with the same name"),
		}
	}
	
	pub async fn anonymous_role(
		&self,
	) -> DbResult<Option<Role>> {
		let anonymous_role = CONFIG.unauthenticated_role.as_ref()
			.or_else(|| CONFIG.default_role.as_ref());
		
		let role_name = match anonymous_role {
			Some(role) => role,
			None => return Ok(None),
		};
		
		let role = role::Entity::find()
			.filter(role::Column::Name.eq(role_name))
			.one(&self.connection).await?;
		
		Ok(role.map(Role::from))
	}
	
	pub async fn anonymous_permissions(&self) -> DbResult<EnumSet<Permission>> {
		let role = self.anonymous_role().await?;
		Ok(role.map(|r| r.permissions.into_iter().collect()).unwrap_or_default())
	}
	
	pub async fn user_permissions(
		&self,
		user: &UserSpecifier,
	) -> DbResult<EnumSet<Permission>> {
		let default_role = CONFIG.default_role.as_ref();
		
		let roles = role::Entity::find()
			.join(sea_orm::JoinType::FullOuterJoin, role::Relation::RoleMember.def())
			.apply_if(default_role.is_none().then_some(()), |q, ()| {
				q.filter(role_member::Column::Member.eq(user.0))
			})
			.apply_if(default_role, |q, role| {
				q.filter(role_member::Column::Member.eq(user.0).or(role::Column::Name.eq(role)))
			})
			.all(&self.connection).await?;
		
		let permissions = roles.into_iter()
			.map(Role::from)
			.flat_map(|r| r.permissions.into_iter())
			.collect();
		
		Ok(permissions)
	}
	
	
	pub async fn list_user_roles(
		&self,
		list: &UserRolesListSpecifier, 
		page: UserRolesPageToken,
		limit: usize,
		filter: RoleFilter,
	) -> DbResult<Page<Reference<Role>>> {
		let default_role = CONFIG.default_role.as_ref();
		
		let roles = role::Entity::find()
			.join(sea_orm::JoinType::FullOuterJoin, role::Relation::RoleMember.def())
			.apply_if(default_role.is_none().then_some(()), |q, ()| {
				q.filter(list.filter())
			})
			.apply_if(default_role, |q, role| {
				q.filter(list.filter().or(role::Column::Name.eq(role)))
			})
			.filter(role::Column::Id.gt(page.0))
			.apply_if(filter.name, |q, name| q.filter(role::Column::Name.like(format!("%{name}%"))))
			.apply_if(filter.icon, |q, icon| q.filter(role::Column::Icon.like(format!("%{icon}%"))))
			.order_by(role::Column::Id, sea_orm::Order::Asc)
			.limit(Some((limit + 1) as _))
			.all(&self.connection).await?;
		
		let next = roles.windows(2).nth(limit.saturating_sub(1))
			.map(|pair| &pair[0])
			.map(UserRolesPageToken::from)
			.map(|token| {
				let uri = list.to_uri();
				let path = uri.path();
				// TODO: filter
				format!("{path}?page={token}&limit={limit}").parse().unwrap()
			});
		
		let items = roles.into_iter()
			.map(Role::from)
			.map(Reference::from)
			.collect();
		
		Ok(Page { items, next, previous: None })
	}
}
