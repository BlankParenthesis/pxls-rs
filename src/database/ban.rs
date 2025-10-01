use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};
use std::fmt;

use sea_orm::sea_query::{Expr, SimpleExpr};
use sea_orm::{Set, NotSet, ColumnTrait, ConnectionTrait, EntityTrait, FromQueryResult, Iden, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect, QueryTrait, RelationTrait, StreamTrait, TransactionTrait};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use warp::http::Uri;

use crate::database::{DbInsertResult, InsertError};
use crate::filter::response::paginated_list::{Page, PageToken};
use crate::filter::response::reference::{Referencable, Reference};
use crate::routes::user_bans::users::BanFilter;

use super::entities::*;
use super::{Connection, DbResult};
use super::user::{User, UserSpecifier};
use super::specifier::{SpecifierParser, Id, SpecfierParseError, PathPart, Specifier, specifier_path};

#[derive(Debug, Eq, PartialEq, Clone, Copy, Hash)]
pub struct BanListSpecifier {
	user: i32,
}

impl BanListSpecifier {
	pub fn user(&self) -> UserSpecifier {
		UserSpecifier(self.user)
	}
	
	pub fn ban(&self, ban: i32) -> BanSpecifier {
		BanSpecifier { user: self.user, ban }
	}
}

impl From<&UserSpecifier> for BanListSpecifier {
	fn from(value: &UserSpecifier) -> Self {
		Self { user: value.0 }
	}
}

impl Specifier for BanListSpecifier {
	fn filter(&self) -> SimpleExpr {
		ban::Column::UserId.eq(self.user)
	}
	
	fn parts() -> &'static [PathPart] {
		specifier_path!("users", user, "bans")
	}
	
	fn from_ids(ids: &[&str]) -> Result<Self, SpecfierParseError> {
		let user = ids[0].parse()?;
		Ok(Self { user })
	}
	
	fn to_ids(&self) -> Box<[Id]> {
		Box::new([Id::I32(self.user)])
	}
}

impl<'de> Deserialize<'de> for BanListSpecifier {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where D: serde::Deserializer<'de> {
		deserializer.deserialize_str(SpecifierParser::new("A ban list uri"))
	}
}

impl Serialize for BanListSpecifier {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where S: serde::Serializer {
		serializer.serialize_str(self.to_uri().path())
	}
}

#[derive(Debug, Eq, PartialEq, Clone, Copy, Hash)]
pub struct BanSpecifier {
	user: i32,
	ban: i32,
}

impl BanSpecifier {
	pub fn user(&self) -> UserSpecifier {
		UserSpecifier(self.user)
	}
	
	pub fn list(&self) -> BanListSpecifier {
		BanListSpecifier { user: self.user }
	}
}

impl Specifier for BanSpecifier {
	fn filter(&self) -> SimpleExpr {
		ban::Column::Id.eq(self.ban)
			.and(ban::Column::UserId.eq(self.user))
	}
	
	fn parts() -> &'static [PathPart] {
		specifier_path!("users", user, "bans", ban)
	}
	
	fn from_ids(ids: &[&str]) -> Result<Self, SpecfierParseError> {
		let user = ids[0].parse()?;
		let ban = ids[1].parse()?;
		Ok(Self { user, ban })
	}
	
	fn to_ids(&self) -> Box<[Id]> {
		Box::new([Id::I32(self.user), Id::I32(self.ban)])
	}
}

impl<'de> Deserialize<'de> for BanSpecifier {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where D: serde::Deserializer<'de> {
		deserializer.deserialize_str(SpecifierParser::new("A ban uri"))
	}
}

impl Serialize for BanSpecifier {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where S: serde::Serializer {
		serializer.serialize_str(self.to_uri().path())
	}
}

#[derive(Debug, Serialize, Clone)]
pub struct Ban {
	#[serde(skip_serializing)]
	id: BanSpecifier,
	pub created_at: u64,
	pub expires_at: Option<u64>,
	pub issuer: Option<Reference<User>>,
	pub reason: Option<String>,
}

impl Ban {
	pub fn specifier(&self) -> &BanSpecifier {
		&self.id
	}
}

impl Referencable for Ban {
	fn uri(&self) -> Uri {
		self.id.to_uri()
	}
}

#[derive(Debug, Default, Deserialize)]
pub struct BanPageToken(pub u32);

impl PageToken for BanPageToken {}

impl fmt::Display for BanPageToken {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{}", self.0)
	}
}

#[derive(Iden)]
enum BanUser {
	Table,
	Subject,
	Name,
	CreatedAt,
}

#[derive(Iden)]
enum BanIssuer {
	Table,
	Subject,
	Name,
	CreatedAt,
}

#[derive(FromQueryResult)]
struct BanFull {
	id: i32,
	created_at: i64,
	expires_at: Option<i64>,
	reason: Option<String>,
	
	user_id: i32,
	user_subject: String,
	user_name: String,
	user_created_at: i64,
	
	issuer: Option<i32>,
	issuer_subject: Option<String>,
	issuer_name: Option<String>,
	issuer_created_at: Option<i64>,
}

impl BanFull {
	fn split(self) -> (ban::Model, user::Model, Option<user::Model>) {
		let ban = ban::Model {
			id: self.id,
			created_at: self.created_at,
			expires_at: self.expires_at,
			reason: self.reason,
			user_id: self.user_id,
			issuer: self.issuer,
		};
		
		let user = user::Model {
			id: self.user_id,
			subject: self.user_subject,
			name: self.user_name,
			created_at: self.user_created_at,
		};
		
		let issuer = self.issuer.map(|id| user::Model {
			id,
			subject: self.issuer_subject.unwrap(),
			name: self.issuer_name.unwrap(),
			created_at: self.issuer_created_at.unwrap(),
		});
		
		(ban, user, issuer)
	}
}

#[derive(Default, Debug, Clone, Copy)]
enum BanStatus {
	#[default]
	NotBanned,
	BannedUntil(u64),
	Permabanned,
}

impl BanStatus {
	async fn from_db<Connection: TransactionTrait + ConnectionTrait + StreamTrait>(
		bans: &BanListSpecifier,
		connection: &Connection
	) -> DbResult<Self> {
		let permanent_ban_count = ban::Entity::find()
			.filter(bans.filter())
			.filter(ban::Column::ExpiresAt.is_null())
			.count(connection).await?;
		let largest_expiry_query = ban::Entity::find()
			.select_only()
			.column_as(ban::Column::ExpiresAt.max(), "expiry")
			.filter(bans.filter())
			.build(connection.get_database_backend());
		let largest_expiry = connection.query_one(largest_expiry_query).await?
			.unwrap() // max always returns one row and will be null if there were no rows
			.try_get::<Option<i64>>("", "expiry")?;

		if permanent_ban_count > 0 {
			Ok(Self::Permabanned)
		} else if let Some(expiry) = largest_expiry {
			Ok(Self::BannedUntil(expiry as _))
		} else {
			Ok(Self::NotBanned)
		}
	}
}

#[derive(Default)]
struct BansCache {
	bans: RwLock<HashMap<UserSpecifier, BanStatus>>,
}

impl BansCache {
	async fn check<Connection: TransactionTrait + ConnectionTrait + StreamTrait>(
		&self,
		user: &UserSpecifier,
		connection: &Connection
	) -> DbResult<BanStatus> {
		let bans = self.bans.read().await;
		if let Some(&ban) = bans.get(user) {
			Ok(ban)
		} else {
			drop(bans);
			let mut bans = self.bans.write().await;
			let list = BanListSpecifier::from(user);
			let status = BanStatus::from_db(&list, connection).await?;
			bans.insert(*user, status);
			Ok(status)
		}
	}

	async fn invalidate(&self, user: &UserSpecifier) {
		self.bans.write().await.remove(user);
	}
}

lazy_static! {
	static ref BANS_CACHE: BansCache = BansCache::default();
}

impl<C: TransactionTrait + ConnectionTrait + StreamTrait> Connection<C> {
	
	pub async fn is_user_banned(&self, user: &UserSpecifier) -> DbResult<bool> {
		let status = BANS_CACHE.check(user, &self.connection).await?;
		match status {
			BanStatus::NotBanned => Ok(false),
			BanStatus::Permabanned => Ok(true),
			BanStatus::BannedUntil(time) => {
				// TODO: make now some shared function somewhere
				let now = SystemTime::now()
					.duration_since(UNIX_EPOCH)
					.unwrap()
					.as_secs();

				Ok(now <= time)
			},
		}
	}

	pub async fn list_bans(
		&self,
		list: &BanListSpecifier,
		token: BanPageToken,
		limit: usize,
		filter: BanFilter,
	) -> DbResult<Page<Reference<Ban>>> {
		
		let bans = ban::Entity::find()
			.column_as(Expr::col((BanUser::Table, BanUser::Subject)), "user_subject")
			.column_as(Expr::col((BanUser::Table, BanUser::Name)), "user_name")
			.column_as(Expr::col((BanUser::Table, BanUser::CreatedAt)), "user_created_at")
			.column_as(Expr::col((BanUser::Table, BanIssuer::Subject)), "issuer_subject")
			.column_as(Expr::col((BanUser::Table, BanIssuer::Name)), "issuer_name")
			.column_as(Expr::col((BanUser::Table, BanIssuer::CreatedAt)), "issuer_created_at")
			.join_as(sea_orm::JoinType::InnerJoin, ban::Relation::User.def(), BanUser::Table)
			.join_as(sea_orm::JoinType::LeftJoin, ban::Relation::Issuer.def(), BanIssuer::Table)
			.filter(ban::Column::Id.gt(token.0))
			.filter(list.filter())
			.apply_if(filter.created_at.start, |q, start| {
				if filter.created_at.end.is_none() {
					q.filter(ban::Column::CreatedAt.gte(start).or(ban::Column::CreatedAt.is_null()))
				} else {
					q.filter(ban::Column::CreatedAt.gte(start))
				}
			})
			.apply_if(filter.created_at.end, |q, end| q.filter(ban::Column::CreatedAt.lte(end)))
			.apply_if(filter.expires_at.start, |q, start| {
				if filter.expires_at.end.is_none() {
					q.filter(ban::Column::ExpiresAt.gte(start).or(ban::Column::ExpiresAt.is_null()))
				} else {
					q.filter(ban::Column::ExpiresAt.gte(start))
				}
			})
			.apply_if(filter.expires_at.end, |q, end| q.filter(ban::Column::ExpiresAt.lte(end)))
			.order_by_asc(ban::Column::Id)
			.limit(Some(limit as u64 + 1))
			.into_model::<BanFull>()
			.all(&self.connection).await?;

		let next = bans.windows(2).nth(limit.saturating_sub(1))
			.map(|pair| &pair[0])
			.map(|ban| BanPageToken(ban.id as _))
			.map(|token| {
				// TODO: filter
				let uri = list.to_uri();
				let path = uri.path();
				format!("{path}?page={token}&limit={limit}").parse().unwrap()
			});
			
		let mut items = Vec::with_capacity(bans.len());
		for ban in bans.into_iter().take(limit) {
			let (ban, user, issuer) = ban.split();
			let ban = Ban {
				id: list.ban(ban.id),
				created_at: ban.created_at as _,
				expires_at: ban.expires_at.map(|e| e as _),
				issuer: issuer.map(User::from).map(Reference::from),
				reason: ban.reason,
			};

			let reference = Reference::from(ban);

			items.push(reference);
		}

		Ok(Page { items, next, previous: None })
	}

	pub async fn get_ban(&self, ban: &BanSpecifier) -> DbResult<Option<Ban>> {
		let ban = ban::Entity::find()
			.column_as(Expr::col((BanUser::Table, BanUser::Subject)), "user_subject")
			.column_as(Expr::col((BanUser::Table, BanUser::Name)), "user_name")
			.column_as(Expr::col((BanUser::Table, BanUser::CreatedAt)), "user_created_at")
			.column_as(Expr::col((BanUser::Table, BanIssuer::Subject)), "issuer_subject")
			.column_as(Expr::col((BanUser::Table, BanIssuer::Name)), "issuer_name")
			.column_as(Expr::col((BanUser::Table, BanIssuer::CreatedAt)), "issuer_created_at")
			.join_as(sea_orm::JoinType::InnerJoin, ban::Relation::User.def(), BanUser::Table)
			.join_as(sea_orm::JoinType::LeftJoin, ban::Relation::Issuer.def(), BanIssuer::Table)
			.filter(ban.filter())
			.order_by_asc(ban::Column::Id)
			.limit(1)
			.into_model::<BanFull>()
			.one(&self.connection).await?
			.map(|b| b.split())
			.map(|(ban, user, issuer)| Ban {
				id: BanSpecifier { user: ban.user_id, ban: ban.id },
				created_at: ban.created_at as _,
				expires_at: ban.expires_at.map(|e| e as _),
				issuer: issuer.map(User::from).map(Reference::from),
				reason: ban.reason,
			});
		
		Ok(ban)
	}

	pub async fn create_ban(
		&self,
		list: &BanListSpecifier,
		issuer: Option<&UserSpecifier>,
		reason: Option<String>,
		expiry: Option<u64>,
	) -> DbInsertResult<Ban> {
		let now = SystemTime::now()
			.duration_since(UNIX_EPOCH)
			.unwrap()
			.as_secs();
		
		let ban = ban::ActiveModel {
			id: NotSet,
			user_id: Set(list.user),
			created_at: Set(now as _),
			expires_at: Set(expiry.map(|e| e as _)),
			issuer: Set(issuer.map(|i| i.0)),
			reason: Set(reason),
		};
		
		let transaction = self.begin().await?;
		
		let user = transaction.get_user(&list.user()).await?
			.ok_or(InsertError::MissingDependency)?;
		
		let issuer = if let Some(issuer) = issuer {
			let user = transaction.get_user(issuer).await?
				.map(Reference::from)
				.ok_or(InsertError::MissingDependency)?;
			Some(user)
		} else {
			None
		};
		
		let insert = ban::Entity::insert(ban)
			.exec_with_returning(&transaction.connection).await?;
		
		transaction.commit().await?;
		BANS_CACHE.invalidate(user.specifier()).await;

		let ban = Ban {
			id: list.ban(insert.id),
			created_at: insert.created_at as _,
			expires_at: insert.expires_at.map(|i| i as _),
			issuer,
			reason: insert.reason,
		};
		Ok(ban)
	}

	pub async fn edit_ban(
		&self,
		ban: &BanSpecifier,
		reason: Option<Option<String>>,
		expiry: Option<Option<u64>>,
	) -> DbResult<Option<Ban>> {
		let model = ban::ActiveModel {
			id: Set(ban.ban),
			user_id: NotSet,
			created_at: NotSet,
			expires_at: expiry.map(|ex| Set(ex.map(|e| e as _))).unwrap_or(NotSet),
			issuer: NotSet,
			reason: reason.map(Set).unwrap_or(NotSet),
		};
		
		let transaction = self.begin().await?;
		
		let user = match transaction.get_user(&ban.user()).await? {
			Some(user) => user,
			None => return Ok(None),
		};
		
		let update = ban::Entity::update(model)
			.filter(ban.filter())
			.exec(&transaction.connection).await?;
		
		let issuer = if let Some(issuer) = update.issuer {
			let issuer = UserSpecifier(issuer);
			let user = transaction.get_user(&issuer).await?
				.expect("failed to lookup ban issuer");
			Some(Reference::from(user))
		} else {
			None
		};
		
		transaction.commit().await?;
		
		BANS_CACHE.invalidate(user.specifier()).await;

		let ban = Ban {
			id: BanSpecifier { user: update.user_id, ban: update.id },
			created_at: update.created_at as _,
			expires_at: update.expires_at.map(|i| i as _),
			issuer,
			reason: update.reason,
		};

		Ok(Some(ban))
	}

	pub async fn delete_ban(
		&self,
		ban: &BanSpecifier,
	) -> DbResult<bool> {
		let delete = ban::Entity::delete_many()
			.filter(ban.filter())
			.exec(&self.connection).await?;

		BANS_CACHE.invalidate(&ban.user()).await;

		Ok(delete.rows_affected > 0)
	}

}
