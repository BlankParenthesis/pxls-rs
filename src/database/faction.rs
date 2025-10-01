use std::time::{SystemTime, UNIX_EPOCH};
use std::fmt;

use sea_orm::sea_query::SimpleExpr;
use sea_orm::{ActiveValue::NotSet, ColumnTrait, ConnectionTrait, EntityTrait, FromQueryResult, QueryFilter, QueryOrder, QuerySelect, QueryTrait, RelationTrait, Set, StreamTrait, TransactionTrait};
use sea_query::{ColumnRef, IntoIden};
use serde::{Deserialize, Serialize};
use url::Url;
use warp::http::Uri;

use crate::database::FactionMemberListSpecifier;
use crate::filter::response::paginated_list::{Page, PageToken};
use crate::filter::response::reference::{Referencable, Reference};
use crate::routes::factions::factions::FactionFilter;

use super::entities::*;
use super::specifier::{SpecifierParser, Id, SpecfierParseError, Specifier, PathPart, specifier_path};
use super::{Connection, DbResult, DatabaseError};

#[derive(Debug, Clone, Copy)]
pub struct FactionListSpecifier;

impl Specifier for FactionListSpecifier {
	fn filter(&self) -> SimpleExpr {
		unimplemented!()
	}
	
	fn from_ids(_: &[&str]) -> Result<Self, SpecfierParseError> {
		Ok(Self)
	}
	
	fn to_ids(&self) -> Box<[Id]> {
		Box::new([])
	}
	
	fn parts() -> &'static [PathPart] {
		specifier_path!("factions")
	}
}

impl<'de> Deserialize<'de> for FactionListSpecifier {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where D: serde::Deserializer<'de> {
		deserializer.deserialize_str(SpecifierParser::new("A faction list uri"))
	}
}

impl Serialize for FactionListSpecifier {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where S: serde::Serializer {
		serializer.serialize_str(self.to_uri().path())
	}
}

#[derive(Debug, Clone, Copy)]
pub struct FactionSpecifier(pub(super) i32);

impl FactionSpecifier {
	pub fn members(&self) -> FactionMemberListSpecifier {
		FactionMemberListSpecifier { faction: self.0 }
	}
}

impl Specifier for FactionSpecifier {
	fn filter(&self) -> SimpleExpr {
		faction::Column::Id.eq(self.0)
	}
	
	fn from_ids(ids: &[&str]) -> Result<Self, SpecfierParseError> {
		let faction = ids[0].parse()?;
		Ok(Self(faction))
	}
	
	fn to_ids(&self) -> Box<[Id]> {
		Box::new([Id::I32(self.0)])
	}
	
	fn parts() -> &'static [PathPart] {
		specifier_path!("factions", faction)
	}
}

impl<'de> Deserialize<'de> for FactionSpecifier {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where D: serde::Deserializer<'de> {
		deserializer.deserialize_str(SpecifierParser::new("A faction uri"))
	}
}

impl Serialize for FactionSpecifier {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where S: serde::Serializer {
		serializer.serialize_str(self.to_uri().path())
	}
}

#[derive(Debug, FromQueryResult)]
pub struct FactionFull {
	pub(super) id: i32,
	pub(super) name: String,
	pub(super) icon: Option<String>,
	pub(super) created_at: i64,
	pub(super) size: i64,
}

impl FactionFull {
	fn from_model_and_size(model: faction::Model, size: i64) -> Self {
		let faction::Model { id, name, icon, created_at } = model;
		FactionFull { id, name, icon, created_at, size }
	}
}

#[derive(Debug, Clone, Serialize)]
pub struct Faction {
	#[serde(skip_serializing)]
	id: FactionSpecifier,
	pub name: String,
	pub icon: Option<Url>,
	pub created_at: i64,
	pub size: usize,
}

impl Faction {
	pub fn specifier(&self) -> &FactionSpecifier {
		&self.id
	}
}

impl Referencable for Faction {
	fn uri(&self) -> Uri {
		self.id.to_uri()
	}
}

impl From<FactionFull> for Faction {
	fn from(faction: FactionFull) -> Self {
		let FactionFull { id, name, icon, created_at, size } = faction;
		let id = FactionSpecifier(id);
		// silently drops invalid icon urls
		let icon = icon.and_then(|i| i.parse().ok());
		let size = size as usize;
		Faction { id, name, icon, created_at, size }
	}
}

#[derive(Debug, Default, Deserialize)]
pub struct FactionsPageToken(pub u32);
impl PageToken for FactionsPageToken {}
impl From<&FactionFull> for FactionsPageToken {
	fn from(value: &FactionFull) -> Self {
		Self(value.id as _)
	}
}
impl fmt::Display for FactionsPageToken {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{}", self.0)
	}
}

impl<C: TransactionTrait + ConnectionTrait + StreamTrait> Connection<C> {
	
	
	pub async fn list_factions(
		&self,
		page: FactionsPageToken,
		limit: usize,
		filter: FactionFilter,
	) -> DbResult<Page<Reference<Faction>>> {
		let factions = faction::Entity::find()
			.column_as(faction_member::Column::Member.count(), "size")
			.join(sea_orm::JoinType::FullOuterJoin, faction::Relation::FactionMember.def())
			.group_by(faction::Column::Id)
			.filter(faction::Column::Id.gt(page.0))
			.apply_if(filter.name, |q, name| {
				let query_builder = self.connection
					.get_database_backend()
					.get_query_builder();
				let filter = name.split('*')
					.map(|part| query_builder.escape_string(part))
					.collect::<Vec<_>>()
					.join("%");
				// TODO: apply same filtering logic to other searches and also do case insenstivity
				q.filter(faction::Column::Name.like(filter))
			})
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
			.order_by(faction::Column::Id, sea_orm::Order::Asc)
			.limit(Some((limit + 1) as u64))
			.into_model::<FactionFull>()
			.all(&self.connection).await?;
		
		let next = factions.windows(2).nth(limit.saturating_sub(1))
			.map(|pair| &pair[0])
			.map(FactionsPageToken::from)
			.map(|token| {
				// TODO: filter
				format!("/factions?page={token}&limit={limit}").parse().unwrap()
			});
		
		let items = factions.into_iter()
			.take(limit)
			.map(Faction::from)
			.map(Reference::from)
			.collect();
		
		// TODO: previous
		Ok(Page { items, next, previous: None })
	}
	
	pub async fn get_faction(
		&self,
		faction: &FactionSpecifier,
	) -> DbResult<Option<Faction>> {
		faction::Entity::find()
			.column_as(faction_member::Column::Member.count(), "size")
			.join(sea_orm::JoinType::FullOuterJoin, faction::Relation::FactionMember.def())
			.group_by(faction::Column::Id)
			.filter(faction.filter())
			.into_model::<FactionFull>()
			.one(&self.connection).await
			.map(|r| r.map(Faction::from))
			.map_err(DatabaseError::from)
	}
	
	pub async fn create_faction(
		&self,
		name: String,
		icon: Option<Url>,
	) -> DbResult<Faction> {
		let icon = icon.map(String::from);
		
		let now = SystemTime::now()
			.duration_since(UNIX_EPOCH)
			.unwrap()
			.as_secs();
		
		let faction = faction::ActiveModel { 
			name: Set(name),
			icon: Set(icon),
			created_at: Set(now as _),
			..Default::default()
		};
		
		let insert = faction::Entity::insert(faction)
			.exec_with_returning(&self.connection).await?;
		
		let faction_full = FactionFull::from_model_and_size(insert, 0);
		
		Ok(Faction::from(faction_full))
	}
	
	pub async fn update_faction(
		&self,
		faction: &FactionSpecifier,
		name: Option<String>,
		icon: Option<Option<Url>>,
	) -> DbResult<Option<Faction>> {
		let model = faction::ActiveModel { 
			name: name.map(Set).unwrap_or(NotSet),
			icon: icon.map(|icon| Set(icon.map(String::from))).unwrap_or(NotSet),
			..Default::default()
		};
		
		let transaction = self.connection.begin().await?;
		// TODO: this doesn't seem right, but just update has no fail state for not found it seems
		let update = faction::Entity::update_many()
			.set(model)
			.filter(faction.filter())
			.exec_with_returning(&transaction).await?;
		
		match update.as_slice() {
			[] => Ok(None),
			[_] => {
				let faction = faction::Entity::find()
					.column_as(faction_member::Column::Member.count(), "size")
					.join(sea_orm::JoinType::FullOuterJoin, faction::Relation::FactionMember.def())
					.group_by(faction_member::Column::Member)
					.filter(faction.filter())
					.into_model::<FactionFull>()
					.one(&transaction).await?
					.expect("updated a faction which disappeared in a transaction");
				
				transaction.commit().await?;
				Ok(Some(Faction::from(faction)))
			},
			_ => panic!("updated multiple factions with the same id"),
		}
	}
	
	pub async fn delete_faction(
		&self,
		faction: &FactionSpecifier,
	) -> DbResult<Option<()>> {
		let transaction = self.begin().await?;
		
		faction_member::Entity::delete_many()
			.filter(faction_member::Column::Faction.eq(faction.0))
			.exec(&transaction.connection).await?;
		
		let delete = faction::Entity::delete_many()
			.filter(faction.filter())
			.exec(&transaction.connection).await?;
		
		match delete.rows_affected {
			0 => Ok(None),
			1 => {
				transaction.commit().await?;
				Ok(Some(()))
			},
			_ => panic!("deleted multiple factions with the same id"),
		}
	}
}
