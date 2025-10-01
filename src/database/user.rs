use std::time::{SystemTime, UNIX_EPOCH};
use std::fmt;

use sea_orm::sea_query::SimpleExpr;
use sea_orm::{Set, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect, QueryTrait, StreamTrait, TransactionTrait};
use serde::{Deserialize, Serialize};
use warp::http::Uri;

use crate::filter::response::paginated_list::{Page, PageToken};
use crate::filter::response::reference::{Referencable, Reference};
use crate::routes::users::users::UserFilter;

use super::entities::*;
use super::specifier::{PathPart, Specifier, SpecifierParser, SpecfierParseError, Id, specifier_path};
use super::{Connection, DbResult};

#[derive(Debug, Eq, PartialEq, Clone, Copy, Hash)]
pub struct UserSpecifier(pub(super) i32);

impl Specifier for UserSpecifier {
	fn filter(&self) -> SimpleExpr {
		user::Column::Id.eq(self.0)
	}
	
	fn from_ids(ids: &[&str]) -> Result<Self, SpecfierParseError> {
		let user = ids[0].parse()?;
		Ok(Self(user))
	}
	
	fn to_ids(&self) -> Box<[Id]> {
		Box::new([Id::I32(self.0)])
	}

	fn parts() -> &'static [PathPart] {
		specifier_path!("users", user)
	}
}

impl<'de> Deserialize<'de> for UserSpecifier {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where D: serde::Deserializer<'de> {
		deserializer.deserialize_str(SpecifierParser::new("A user uri"))
	}
}

impl Serialize for UserSpecifier {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where S: serde::Serializer {
		serializer.serialize_str(self.to_uri().path())
	}
}

#[derive(Debug, Eq, PartialEq, Clone, Copy, Hash)]
pub struct UserStatsSpecifier(pub(super) i32);

impl UserStatsSpecifier {
	pub fn user(&self) -> UserSpecifier {
		UserSpecifier(self.0)
	}
}

impl Specifier for UserStatsSpecifier {
	fn filter(&self) -> SimpleExpr {
		user::Column::Id.eq(self.0)
	}
	
	fn from_ids(ids: &[&str]) -> Result<Self, SpecfierParseError> {
		let user = ids[0].parse()?;
		Ok(Self(user))
	}
	
	fn to_ids(&self) -> Box<[Id]> {
		Box::new([Id::I32(self.0)])
	}

	fn parts() -> &'static [PathPart] {
		specifier_path!("users", user, "stats")
	}
}

impl<'de> Deserialize<'de> for UserStatsSpecifier {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where D: serde::Deserializer<'de> {
		deserializer.deserialize_str(SpecifierParser::new("A user stats uri"))
	}
}

impl Serialize for UserStatsSpecifier {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where S: serde::Serializer {
		serializer.serialize_str(self.to_uri().path())
	}
}

#[derive(Debug, Serialize, Clone)]
pub struct User {
	#[serde(skip_serializing)]
	id: UserSpecifier,
	#[serde(skip_serializing)]
	pub subject: String,
	pub name: String,
	pub created_at: i64,
}

impl User {
	pub fn specifier(&self) -> &UserSpecifier {
		&self.id
	}
}

impl Referencable for User {
	fn uri(&self) -> Uri {
		self.id.to_uri()
	}
}

impl From<user::Model> for User {
	fn from(user: user::Model) -> Self {
		let user::Model { id, subject, name, created_at } = user;
		let id = UserSpecifier(id);
		User { id, subject, name, created_at }
	}
}

#[derive(Debug, Default, Deserialize)]
pub struct UsersPageToken(pub u32);
impl PageToken for UsersPageToken {}
impl From<&user::Model> for UsersPageToken {
	fn from(value: &user::Model) -> Self {
		Self(value.id as _)
	}
}
impl fmt::Display for UsersPageToken {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{}", self.0)
	}
}

impl<C: TransactionTrait + ConnectionTrait + StreamTrait> Connection<C> {
	
	pub async fn list_users(
		&self,
		page: UsersPageToken,
		limit: usize,
		filter: UserFilter,
	) -> DbResult<Page<Reference<User>>> {
		let users = user::Entity::find()
			.filter(user::Column::Id.gt(page.0))
			.apply_if(filter.name, |q, name| q.filter(user::Column::Name.like(format!("%{name}%"))))
			.apply_if(filter.created_at.start, |q, start| {
				if filter.created_at.end.is_none() {
					q.filter(user::Column::CreatedAt.gte(start).or(user::Column::CreatedAt.is_null()))
				} else {
					q.filter(user::Column::CreatedAt.gte(start))
				}
			})
			.apply_if(filter.created_at.end, |q, end| q.filter(user::Column::CreatedAt.lte(end)))
			.order_by(user::Column::Id, sea_orm::Order::Asc)
			.limit(Some((limit + 1) as u64))
			.all(&self.connection).await?;
		
		let next = users.windows(2).nth(limit.saturating_sub(1))
			.map(|pair| &pair[0])
			.map(UsersPageToken::from)
			.map(|token| {
				 // TODO: filter
				format!("/users?page={token}&limit={limit}").parse().unwrap()
			});
		
		let items = users.into_iter()
			.take(limit)
			.map(User::from)
			.map(Reference::from)
			.collect();
		
		// TODO: previous
		Ok(Page { items, next, previous: None })
	}
	
	pub async fn create_user(
		&self,
		subject: String,
		username: String,
		created_at: SystemTime,
	) -> DbResult<User> {
		let created_at = created_at
			.duration_since(UNIX_EPOCH)
			.unwrap()
			.as_secs();
		
		let model = user::ActiveModel {
			subject: Set(subject.clone()),
			name: Set(username),
			created_at: Set(created_at as i64),
			..Default::default()
		};
		
		let transaction = self.begin().await?;
		
		let user = user::Entity::find()
			.filter(user::Column::Subject.eq(subject))
			.one(&transaction.connection).await?;
		
		if let Some(existing) = user {
			transaction.commit().await?;
			Ok(User::from(existing))
		} else {
			let insert = user::Entity::insert(model)
				.exec_with_returning(&transaction.connection).await?;
			
			transaction.commit().await?;
			Ok(User::from(insert))
		}
	}
	
	pub async fn get_user(
		&self,
		user: &UserSpecifier,
	) -> DbResult<Option<User>> {
		let user = user::Entity::find()
			.filter(user.filter())
			.one(&self.connection).await?;
		
		Ok(user.map(User::from))
	}
	
	pub async fn update_user(
		&self,
		user: &UserSpecifier,
		name: &str,
	) -> DbResult<Option<User>> {
		let model = user::ActiveModel {
			name: Set(name.to_owned()),
			..Default::default()
		};
		
		let update = user::Entity::update_many()
			.set(model)
			.filter(user.filter())
			.exec_with_returning(&self.connection).await?;
		
		match update.as_slice() {
			[] => Ok(None),
			[user] => Ok(Some(User::from(user.clone()))),
			_ => panic!("updated multiple users with the same subject"),
		}
	}
	
	pub async fn delete_user(
		&self,
		user: &UserSpecifier,
	) -> DbResult<Option<()>> {
		let model = user::ActiveModel {
			id: Set(user.0),
			..Default::default()
		};
		
		let delete = user::Entity::delete(model)
			.exec(&self.connection).await?;
		
		match delete.rows_affected {
			0 => Ok(None),
			1 => Ok(Some(())),
			_ => panic!("deleted multiple users with the same subject"),
		}
	}
}
