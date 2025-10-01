use std::fmt;

use sea_orm::sea_query::{Expr, SimpleExpr};
use sea_orm::TryInsertResult;
use sea_orm::{ActiveValue::NotSet, ColumnTrait, ConnectionTrait, EntityTrait, FromQueryResult, QueryFilter, QueryOrder, QuerySelect, QueryTrait, RelationTrait, Set, StreamTrait, TransactionTrait};
use sea_query::{ColumnRef, IntoIden};
use serde::{Deserialize, Serialize};
use warp::http::Uri;

use crate::database::faction::FactionSpecifier;
use crate::database::user::UserSpecifier;
use crate::database::{DbInsertResult, InsertError};
use crate::filter::response::paginated_list::{Page, PageToken};
use crate::filter::response::reference::{Referencable, Reference};
use crate::routes::factions::factions::members::FactionMemberFilter;
use crate::routes::factions::factions::FactionFilter;

use super::entities::*;
use super::specifier::{SpecifierParser, Id, SpecfierParseError, Specifier, PathPart, specifier_path};
use super::faction::{Faction, FactionFull};
use super::user::User;
use super::{Connection, DbResult};

#[derive(Debug, Eq, PartialEq, Clone, Copy, Hash)]
pub struct UserFactionMemberListSpecifier {
	user: i32,
}

impl UserFactionMemberListSpecifier {
	pub fn user(&self) -> UserSpecifier {
		UserSpecifier(self.user)
	}
	
	fn member(&self, faction: i32) -> FactionMemberSpecifier {
		FactionMemberSpecifier { faction, member: self.user }
	}
}

impl Specifier for UserFactionMemberListSpecifier {
	fn filter(&self) -> SimpleExpr {
		faction_member::Column::Member.eq(self.user)
	}
	
	fn parts() -> &'static [PathPart] {
		specifier_path!("users", user, "factions")
	}
	
	fn from_ids(ids: &[&str]) -> Result<Self, SpecfierParseError> {
		let user = ids[0].parse()?;
		Ok(Self { user })
	}
	
	fn to_ids(&self) -> Box<[Id]> {
		Box::new([Id::I32(self.user)])
	}
}

impl<'de> Deserialize<'de> for UserFactionMemberListSpecifier {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where D: serde::Deserializer<'de> {
		deserializer.deserialize_str(SpecifierParser::new("A user faction member list uri"))
	}
}

impl Serialize for UserFactionMemberListSpecifier {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where S: serde::Serializer {
		serializer.serialize_str(self.to_uri().path())
	}
}

#[derive(Debug, Eq, PartialEq, Clone, Copy, Hash)]
pub struct FactionMemberListSpecifier {
	pub(super) faction: i32,
}

impl FactionMemberListSpecifier {
	fn faction(&self) -> FactionSpecifier {
		FactionSpecifier(self.faction)
	}
	
	fn member(&self, member: i32) -> FactionMemberSpecifier {
		FactionMemberSpecifier { faction: self.faction, member }
	}
}

impl Specifier for FactionMemberListSpecifier {
	fn filter(&self) -> SimpleExpr {
		faction_member::Column::Faction.eq(self.faction)
	}
	
	fn parts() -> &'static [PathPart] {
		specifier_path!("factions", faction, "members")
	}
	
	fn from_ids(ids: &[&str]) -> Result<Self, SpecfierParseError> {
		let faction = ids[0].parse()?;
		Ok(Self { faction })
	}
	
	fn to_ids(&self) -> Box<[Id]> {
		Box::new([Id::I32(self.faction)])
	}
}

impl<'de> Deserialize<'de> for FactionMemberListSpecifier {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where D: serde::Deserializer<'de> {
		deserializer.deserialize_str(SpecifierParser::new("A faction member list uri"))
	}
}

impl Serialize for FactionMemberListSpecifier {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where S: serde::Serializer {
		serializer.serialize_str(self.to_uri().path())
	}
}

#[derive(Debug, Eq, PartialEq, Clone, Copy, Hash)]
pub struct FactionMemberCurrentSpecifier {
	pub(super) faction: i32,
}

impl FactionMemberCurrentSpecifier {
	pub fn faction(&self) -> FactionSpecifier {
		FactionSpecifier(self.faction)
	}
	
	pub fn member(&self, user: &UserSpecifier) -> FactionMemberSpecifier {
		FactionMemberSpecifier { faction: self.faction, member: user.0 }
	}
}

impl Specifier for FactionMemberCurrentSpecifier {
	fn filter(&self) -> SimpleExpr {
		faction_member::Column::Faction.eq(self.faction)
	}
	
	fn parts() -> &'static [PathPart] {
		specifier_path!("factions", faction, "members", "current")
	}
	
	fn from_ids(ids: &[&str]) -> Result<Self, SpecfierParseError> {
		let faction = ids[0].parse()?;
		Ok(Self { faction })
	}
	
	fn to_ids(&self) -> Box<[Id]> {
		Box::new([Id::I32(self.faction)])
	}
}

impl<'de> Deserialize<'de> for FactionMemberCurrentSpecifier {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where D: serde::Deserializer<'de> {
		deserializer.deserialize_str(SpecifierParser::new("A faction member uri"))
	}
}

impl Serialize for FactionMemberCurrentSpecifier {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where S: serde::Serializer {
		serializer.serialize_str(self.to_uri().path())
	}
}

#[derive(Debug, Eq, PartialEq, Clone, Copy, Hash)]
pub struct FactionMemberSpecifier {
	faction: i32,
	member: i32,
}

impl FactionMemberSpecifier {
	pub fn faction(&self) -> FactionSpecifier {
		FactionSpecifier(self.faction)
	}
	
	pub fn user(&self) -> UserSpecifier {
		UserSpecifier(self.member)
	}
	
	pub fn list(&self) -> FactionMemberListSpecifier {
		FactionMemberListSpecifier { faction: self.faction }
	}
}

impl Specifier for FactionMemberSpecifier {
	fn filter(&self) -> SimpleExpr {
		faction_member::Column::Faction.eq(self.faction)
			.and(faction_member::Column::Member.eq(self.member))
	}
	
	fn parts() -> &'static [PathPart] {
		specifier_path!("factions", faction, "members", member)
	}
	
	fn from_ids(ids: &[&str]) -> Result<Self, SpecfierParseError> {
		let faction = ids[0].parse()?;
		let member = ids[1].parse()?;
		Ok(Self { faction, member })
	}
	
	fn to_ids(&self) -> Box<[Id]> {
		Box::new([Id::I32(self.faction), Id::I32(self.member)])
	}
}

impl<'de> Deserialize<'de> for FactionMemberSpecifier {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where D: serde::Deserializer<'de> {
		deserializer.deserialize_str(SpecifierParser::new("A faction member uri"))
	}
}

impl Serialize for FactionMemberSpecifier {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where S: serde::Serializer {
		serializer.serialize_str(self.to_uri().path())
	}
}

#[derive(Clone, Debug, Serialize)]
pub struct JoinIntent {
	pub member: bool,
	pub faction: bool,
}

#[derive(Debug, FromQueryResult)]
pub struct FactionMemberFull {
	invited: bool,
	imposed: bool,
	owner: bool,
	
	faction_id: i32,
	faction_name: String,
	faction_icon: Option<String>,
	faction_created_at: i64,
	faction_size: i64,

	member_id: i32,
	member_subject: String,
	member_name: String,
	member_created_at: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct FactionMember {
	#[serde(skip_serializing)]
	id: FactionMemberSpecifier,
	#[serde(skip_serializing)]
	faction: Faction,
	user: Reference<User>,
	join_intent: JoinIntent,
	owner: bool,
}

impl FactionMember {
	pub fn faction(&self) -> &Faction {
		&self.faction
	}
	
	fn from_model(meta: faction_member::Model, faction: Faction, member: User) -> Self {
		Self {
			id: FactionMemberSpecifier { faction: meta.faction, member: meta.member },
			faction,
			user: Reference::from(member),
			join_intent: JoinIntent {
				member: meta.imposed,
				faction: meta.invited,
			},
			owner: meta.owner,
		}
	}
}

impl Referencable for FactionMember {
	fn uri(&self) -> Uri {
		self.id.to_uri()
	}
}

impl From<FactionMemberFull> for FactionMember {
	fn from(faction_member: FactionMemberFull) -> Self {
		let id = FactionMemberSpecifier {
			faction: faction_member.faction_id,
			member: faction_member.member_id,
		};
		
		let join_intent = JoinIntent {
			faction: faction_member.invited,
			member: faction_member.imposed,
		};
		let owner = faction_member.owner;
		
		let faction = Faction::from(FactionFull {
			id: faction_member.faction_id,
			name: faction_member.faction_name.clone(),
			icon: faction_member.faction_icon.clone(),
			created_at: faction_member.faction_created_at,
			size: faction_member.faction_size,
		});
				
		let user = Reference::from(User::from(user::Model {
			id: faction_member.member_id,
			name: faction_member.member_name,
			subject: faction_member.member_subject,
			created_at: faction_member.member_created_at,
		}));
		
		FactionMember { id, faction, user, join_intent, owner }
	}
}

#[derive(Debug, Default, Deserialize)]
pub struct FactionMembersPageToken(pub u32);
impl PageToken for FactionMembersPageToken {}
impl From<&FactionMemberFull> for FactionMembersPageToken {
	fn from(value: &FactionMemberFull) -> Self {
		Self(value.member_id as _)
	}
}
impl fmt::Display for FactionMembersPageToken {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{}", self.0)
	}
}


#[derive(Debug, Clone, Serialize)]
pub struct UserFactionMember {
	faction: Reference<Faction>,
	member: Reference<FactionMember>,
}

impl UserFactionMember {
	fn user(&self) -> &User {
		&self.member.view.user.view
	}
}

impl From<FactionMemberFull> for UserFactionMember {
	fn from(value: FactionMemberFull) -> Self {
		let faction = Faction::from(FactionFull {
			id: value.faction_id,
			name: value.faction_name.clone(),
			icon: value.faction_icon.clone(),
			created_at: value.faction_created_at,
			size: value.faction_size,
		});
		
		let member = FactionMember::from(value);
		
		Self {
			faction: Reference::from(faction),
			member: Reference::from(member),
		}
	}
}

#[derive(Debug, Default, Deserialize)]
pub struct UserFactionsPageToken(pub u32);
impl PageToken for UserFactionsPageToken {}
impl From<&FactionMemberFull> for UserFactionsPageToken {
	fn from(value: &FactionMemberFull) -> Self {
		Self(value.faction_id as _)
	}
}
impl fmt::Display for UserFactionsPageToken {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{}", self.0)
	}
}

impl<C: TransactionTrait + ConnectionTrait + StreamTrait> Connection<C> {
	pub async fn list_faction_members(
		&self,
		list: &FactionMemberListSpecifier,
		page: FactionMembersPageToken,
		limit: usize,
		filter: FactionMemberFilter,
	) -> DbResult<Page<Reference<FactionMember>>> {
		let members = faction_member::Entity::find().select_only()
			.tbl_col_as((faction_member::Entity, faction_member::Column::Invited), "invited")
			.tbl_col_as((faction_member::Entity, faction_member::Column::Imposed), "imposed")
			.tbl_col_as((faction_member::Entity, faction_member::Column::Owner), "owner")
			.tbl_col_as((faction::Entity, faction::Column::Id), "faction_id")
			.tbl_col_as((faction::Entity, faction::Column::Name), "faction_name")
			.tbl_col_as((faction::Entity, faction::Column::Icon), "faction_icon")
			.tbl_col_as((faction::Entity, faction::Column::CreatedAt), "faction_created_at")
			.column_as(Expr::col(("member_count".into_iden(), faction_member::Column::Member)).count(), "size")
			.tbl_col_as((user::Entity, user::Column::Id), "member_id")
			.tbl_col_as((user::Entity, user::Column::Subject), "member_subject")
			.tbl_col_as((user::Entity, user::Column::Name), "member_name")
			.tbl_col_as((user::Entity, user::Column::CreatedAt), "member_created_at")
			.inner_join(faction::Entity)
			.inner_join(user::Entity)
			.filter(faction_member::Column::Member.gt(page.0))
			.order_by(faction_member::Column::Member, sea_orm::Order::Asc)
			.apply_if(filter.owner, |q, owner| q.filter(faction_member::Column::Owner.eq(owner)))
			.join_as(sea_orm::JoinType::FullOuterJoin, faction::Relation::FactionMember.def(), "member_count")
			.group_by(Expr::col(("member_count".into_iden(), faction_member::Column::Member)))
			.filter(list.filter())
			.into_model::<FactionMemberFull>()
			.all(&self.connection).await?;
		
		let next = members.windows(2).nth(limit.saturating_sub(1))
			.map(|pair| &pair[0])
			.map(FactionMembersPageToken::from)
			.map(|token| {
				// TODO: filter
				let uri = list.to_uri();
				let path = uri.path();
				format!("{path}?page={token}&limit={limit}").parse().unwrap()
			});
		
		let items = members.into_iter()
			.map(FactionMember::from)
			.map(Reference::from)
			.collect();
		
		Ok(Page { items, next, previous: None })
	}
	
	pub async fn get_faction_member(
		&self,
		member: &FactionMemberSpecifier,
	) -> DbResult<Option<FactionMember>> {
		let transaction = self.begin().await?;
		
		let find = faction_member::Entity::find()
			.filter(member.filter())
			.one(&transaction.connection).await?;
		
		if let Some(model) = find {
			let user = self.get_user(&member.user()).await?
				.expect("failed to find user for faction member");
			
			let faction = self.get_faction(&member.faction()).await?
				.expect("failed to find user for faction member");
			
			transaction.commit().await?;
			
			Ok(Some(FactionMember::from_model(model, faction, user)))
		} else {
			Ok(None)
		}
	}
	
	pub async fn create_faction_member(
		&self,
		members: &FactionMemberListSpecifier,
		member: &UserSpecifier,
		owner: bool,
		invited: bool,
		imposed: bool,
	) -> DbInsertResult<FactionMember> {
		let faction_member = faction_member::ActiveModel {
			faction: Set(members.faction),
			member: Set(member.0),
			owner: Set(owner),
			invited: Set(invited),
			imposed: Set(imposed),
		};
		
		let transaction = self.begin().await?;
		
		let user = transaction.get_user(member).await?
			.map(Reference::from)
			.ok_or(InsertError::MissingDependency)?;
		
		let faction = transaction.get_faction(&members.faction()).await?
			.ok_or(InsertError::MissingDependency)?;
		
		let insert = faction_member::Entity::insert(faction_member)
			.on_conflict_do_nothing()
			.exec(&transaction.connection).await?;
		
		transaction.commit().await?;
		
		match insert {
			TryInsertResult::Inserted(_) => {
				
				let faction_member = FactionMember {
					id: members.member(member.0),
					faction,
					user,
					join_intent: JoinIntent {
						faction: invited,
						member: imposed,
					},
					owner,
				};
				Ok(faction_member)
			},
			TryInsertResult::Conflicted => Err(InsertError::AlreadyExists),
			TryInsertResult::Empty => panic!(),
		}
	}
	
	pub async fn update_faction_member(
		&self,
		member: &FactionMemberSpecifier,
		owner: Option<bool>,
		invited: Option<bool>,
		imposed: Option<bool>,
	) -> DbResult<Option<FactionMember>> {
		let faction_member = faction_member::ActiveModel { 
			faction: Set(member.faction),
			member: Set(member.member),
			owner: owner.map(Set).unwrap_or(NotSet),
			invited: invited.map(Set).unwrap_or(NotSet),
			imposed: imposed.map(Set).unwrap_or(NotSet),
		};
		
		let transaction = self.begin().await?;
		// TODO: this doesn't seem right, but just update has no fail state for not found it seems
		let update = faction_member::Entity::update_many()
			.set(faction_member)
			.filter(member.filter())
			.exec_with_returning(&transaction.connection).await?;
		
		match update.as_slice() {
			[] => Ok(None),
			[faction_member] => {
				let user = transaction.get_user(&member.user()).await?
					.expect("updated faction member but failed to find user");
				
				let faction = transaction.get_faction(&member.faction()).await?
					.expect("updated faction member but failed to find faction");
				
				transaction.commit().await?;
				let faction_member = FactionMember {
					id: *member,
					user: Reference::from(user),
					faction,
					join_intent: JoinIntent {
						faction: faction_member.invited,
						member: faction_member.imposed,
					},
					owner: faction_member.owner,
				};
				Ok(Some(faction_member))
			},
			_ => panic!("updated multiple factions with the same id"),
		}
	}
	
	pub async fn delete_faction_member(
		&self,
		member: &FactionMemberSpecifier,
	) -> DbResult<Option<FactionMember>> {
		let transaction = self.begin().await?;
		
		let faction_member = transaction.get_faction_member(member).await?;
		
		let mut faction_member = match faction_member {
			Some(mut member) => {
				member.join_intent.faction = false;
				member.join_intent.member = false;
				member
			},
			None => return Ok(None),
		};
		
		let delete = faction_member::Entity::delete_many()
			.filter(member.filter())
			.exec(&transaction.connection).await?;
		
		faction_member.faction.size -= 1;
		
		transaction.commit().await?;
		
		match delete.rows_affected {
			0 => Ok(None),
			1 => Ok(Some(faction_member)),
			_ => panic!("deleted multiple faction members with the same keys"),
		}
	}
	
	pub async fn list_user_factions(
		&self,
		list: &UserFactionMemberListSpecifier, 
		page: UserFactionsPageToken,
		limit: usize,
		filter: FactionFilter,
		// TODO: member filter also
	) -> DbResult<Page<UserFactionMember>> {
		let members = faction::Entity::find().select_only()
			.tbl_col_as((faction_member::Entity, faction_member::Column::Invited), "invited")
			.tbl_col_as((faction_member::Entity, faction_member::Column::Imposed), "imposed")
			.tbl_col_as((faction_member::Entity, faction_member::Column::Owner), "owner")
			.tbl_col_as((faction::Entity, faction::Column::Id), "faction_id")
			.tbl_col_as((faction::Entity, faction::Column::Name), "faction_name")
			.tbl_col_as((faction::Entity, faction::Column::Icon), "faction_icon")
			.tbl_col_as((faction::Entity, faction::Column::CreatedAt), "faction_created_at")
			.column_as(Expr::col(("member_count".into_iden(), faction_member::Column::Member)).count(), "faction_size")
			.tbl_col_as((user::Entity, user::Column::Id), "member_id")
			.tbl_col_as((user::Entity, user::Column::Subject), "member_subject")
			.tbl_col_as((user::Entity, user::Column::Name), "member_name")
			.tbl_col_as((user::Entity, user::Column::CreatedAt), "member_created_at")
			// for unknown reasons, this is already included??
			// .inner_join(faction_member::Entity)
			.inner_join(user::Entity)
			.join_as(sea_orm::JoinType::FullOuterJoin, faction::Relation::FactionMember.def(), "member_count")
			.group_by(faction::Column::Id)
			.group_by(user::Column::Id)
			.group_by(faction_member::Column::Invited)
			.group_by(faction_member::Column::Imposed)
			.group_by(faction_member::Column::Owner)
			.group_by(faction_member::Column::Faction)
			.group_by(faction_member::Column::Member)
			.apply_if(filter.name, |q, name| q.filter(faction::Column::Name.like(format!("%{name}%"))))
			.apply_if(filter.created_at.start, |q, start| {
				if filter.created_at.end.is_none() {
					q.filter(faction::Column::CreatedAt.gte(start).or(faction::Column::CreatedAt.is_null()))
				} else {
					q.filter(faction::Column::CreatedAt.gte(start))
				}
			})
			.apply_if(filter.created_at.end, |q, end| q.filter(faction::Column::CreatedAt.lte(end)))
			.apply_if(filter.size.start, |q, start| {
				let expr = SimpleExpr::Binary(
					Box::new(SimpleExpr::Column(ColumnRef::Column("size".into_iden()))),
					sea_query::BinOper::GreaterThanOrEqual,
					Box::new(SimpleExpr::Constant((start as i64).into()))
				);
				q.filter(expr)
			})
			.apply_if(filter.size.end, |q, end| {
				let expr = SimpleExpr::Binary(
					Box::new(SimpleExpr::Column(ColumnRef::Column("size".into_iden()))),
					sea_query::BinOper::SmallerThanOrEqual,
					Box::new(SimpleExpr::Constant((end as i64).into()))
				);
				q.filter(expr)
			})
			.filter(list.filter())
			.filter(faction_member::Column::Faction.gte(page.0))
			.order_by(faction_member::Column::Faction, sea_orm::Order::Asc)
			.into_model::<FactionMemberFull>()
			.all(&self.connection).await?;
		
		let next = members.windows(2).nth(limit.saturating_sub(1))
			.map(|pair| &pair[0])
			.map(UserFactionsPageToken::from)
			.map(|token| {
				// TODO: filter
				let uri = list.to_uri();
				let path = uri.path();
				format!("{path}?page={token}&limit={limit}").parse().unwrap()
			});
		
		let items = members.into_iter()
			.map(UserFactionMember::from)
			.collect();
		
		Ok(Page { items, next, previous: None })
	}
	
	pub async fn all_faction_members(
		&self,
		list: &FactionMemberListSpecifier,
	) -> DbResult<Vec<UserSpecifier>> {
		let members = faction_member::Entity::find()
			.filter(list.filter())
			.all(&self.connection).await?;
		
		let ids = members.into_iter()
			.map(|m| UserSpecifier(m.member))
			.collect();
		
		Ok(ids)
	}
	
	pub async fn all_faction_owners(
		&self,
		list: &FactionMemberListSpecifier,
	) -> DbResult<Vec<UserSpecifier>> {
		let members = faction_member::Entity::find()
			.filter(list.filter())
			.filter(faction_member::Column::Owner.eq(true))
			.all(&self.connection).await?;
		
		let ids = members.into_iter()
			.map(|m| UserSpecifier(m.member))
			.collect();
		
		Ok(ids)
	}
}
