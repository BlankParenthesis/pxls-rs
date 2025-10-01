use std::time::{SystemTime, UNIX_EPOCH};

use sea_orm::sea_query::{Expr, SimpleExpr};
use sea_orm::{ActiveValue::NotSet, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect, QueryTrait, Set, StreamTrait, TransactionTrait};
use serde::{Deserialize, Serialize};
use warp::http::Uri;

use crate::filter::response::paginated_list::Page;
use crate::filter::response::reference::{Referencable, Reference};
use crate::routes::site_notices::notices::NoticePageToken;

use super::entities::*;

use super::{Connection, DbResult, DatabaseError};
use super::user::{User, UserSpecifier};
use super::filter::FilterRange;
use super::specifier::{SpecifierParser, Id, SpecfierParseError, Specifier, PathPart, specifier_path};

#[derive(Debug, Clone, Copy)]
pub struct NoticeSpecifier(i32);

impl Specifier for NoticeSpecifier {
	fn filter(&self) -> SimpleExpr {
		notice::Column::Id.eq(self.0)
	}
	
	fn from_ids(ids: &[&str]) -> Result<Self, SpecfierParseError> {
		let notice = ids[0].parse()?;
		Ok(Self(notice))
	}
	
	fn to_ids(&self) -> Box<[Id]> {
		Box::new([Id::I32(self.0)])
	}
	
	fn parts() -> &'static [PathPart] {
		specifier_path!("notices", notice)
	}
}

impl<'de> Deserialize<'de> for NoticeSpecifier {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where D: serde::Deserializer<'de> {
		deserializer.deserialize_str(SpecifierParser::new("A notice uri"))
	}
}

impl Serialize for NoticeSpecifier {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where S: serde::Serializer {
		serializer.serialize_str(self.to_uri().path())
	}
}

#[derive(Deserialize, Debug)]
pub struct NoticeFilter {
	pub title: Option<String>,
	pub content: Option<String>,
	#[serde(default)]
	pub created_at: FilterRange<u64>,
	#[serde(default)]
	pub expires_at: FilterRange<u64>, // TODO: explicit null?
	pub author: Option<UserSpecifier>,
}

#[serde_with::skip_serializing_none]
#[derive(Serialize, Debug, Clone)]
pub struct Notice {
	#[serde(skip_serializing)]
	id: NoticeSpecifier,
	pub title: String,
	pub content: String,
	pub created_at: u64,
	pub expires_at: Option<u64>,
	pub author: Option<Reference<User>>,
}

impl Referencable for Notice {
	fn uri(&self) -> Uri {
		self.id.to_uri()
	}
}

impl<C: TransactionTrait + ConnectionTrait + StreamTrait> Connection<C> {
	pub async fn list_notices(
		&self,
		token: NoticePageToken,
		limit: usize,
		filter: NoticeFilter,
	) -> DbResult<Page<Reference<Notice>>> {
		let column_timestamp_id_pair = Expr::tuple([
			Expr::col(notice::Column::CreatedAt).into(),
			Expr::col(notice::Column::Id).into(),
		]);

		let value_timestamp_id_pair = Expr::tuple([
			(token.timestamp as i64).into(),
			(token.id as i32).into(),
		]);

		let notices = notice::Entity::find()
			.find_also_related(user::Entity)
			.filter(Expr::gte(column_timestamp_id_pair.clone(), value_timestamp_id_pair))
			.apply_if(filter.author.as_ref(), |q, id| q.filter(notice::Column::Author.eq(id.0)))
			.apply_if(filter.content.as_ref(), |q, content| q.filter(notice::Column::Content.eq(content)))
			.apply_if(filter.title.as_ref(), |q, title| q.filter(notice::Column::Title.eq(title)))
			.apply_if(filter.created_at.start, |q, start| q.filter(notice::Column::CreatedAt.gte(start)))
			.apply_if(filter.created_at.end, |q, end| q.filter(notice::Column::CreatedAt.lte(end)))
			.apply_if(filter.expires_at.start, |q, start| q.filter(notice::Column::ExpiresAt.gte(start)))
			.apply_if(filter.expires_at.end, |q, end| q.filter(notice::Column::ExpiresAt.lte(end)))
			.order_by(column_timestamp_id_pair, sea_orm::Order::Asc)
			.limit(limit as u64 + 1)
			.all(&self.connection).await?;

		let next = notices.windows(2).nth(limit.saturating_sub(1))
			.map(|pair| &pair[0])
			.map(|(notice, _)| NoticePageToken {
				id: notice.id as _,
				timestamp: notice.created_at as _,
			})
			.map(|token| {
				format!( // TODO: filters
					"/notices?page={}&limit={}",
					token, limit,
				).parse().unwrap()
			});

		let notices = notices.into_iter()
			.take(limit)
			.map(|(notice, author)| Notice {
				id: NoticeSpecifier(notice.id),
				title: notice.title,
				content: notice.content,
				created_at: notice.created_at as _,
				expires_at: notice.expires_at.map(|v| v as _),
				author: author.map(User::from).map(Reference::from),
			})
			.map(Reference::from)
			.collect();
		
		Ok(Page { items: notices, next, previous: None })
	}

	pub async fn get_notice(
		&self,
		notice: &NoticeSpecifier,
	) -> DbResult<Option<Notice>> {
		notice::Entity::find_by_id(notice.0)
			.find_also_related(user::Entity)
			.one(&self.connection).await
			.map(|n| n.map(|(notice, author)| Notice {
				id: NoticeSpecifier(notice.id),
				title: notice.title,
				content: notice.content,
				created_at: notice.created_at as _,
				expires_at: notice.expires_at.map(|v| v as _),
				author: author.map(User::from).map(Reference::from),
			}))
			.map_err(DatabaseError::from)
	}

	pub async fn create_notice(
		&self,
		title: String,
		content: String,
		expiry: Option<u64>,
	) -> DbResult<Reference<Notice>> {
		let now = SystemTime::now()
			.duration_since(UNIX_EPOCH)
			.unwrap()
			.as_secs();
		
		let notice = notice::ActiveModel {
			id: NotSet,
			title: Set(title),
			content: Set(content),
			created_at: Set(now as _),
			expires_at: Set(expiry.map(|v| v as _)),
			author: NotSet, // TODO: set this
		};

		notice::Entity::insert(notice)
			.exec_with_returning(&self.connection).await
			.map(|notice| Notice {
				id: NoticeSpecifier(notice.id),
				title: notice.title,
				content: notice.content,
				created_at: notice.created_at as _,
				expires_at: notice.expires_at.map(|v| v as _),
				author: None, // TODO: when this is set, retrieve it
			})
			.map(Reference::from)
			.map_err(DatabaseError::from)
	}

	pub async fn edit_notice(
		&self,
		notice: &NoticeSpecifier,
		title: Option<String>,
		content: Option<String>,
		expiry: Option<Option<u64>>,
	) -> DbResult<Reference<Notice>> {
		let notice = notice::ActiveModel {
			id: Set(notice.0),
			title: title.map(Set).unwrap_or(NotSet),
			content: content.map(Set).unwrap_or(NotSet),
			created_at: NotSet,
			expires_at: expiry.map(|e| Set(e.map(|v| v as _))).unwrap_or(NotSet),
			author: NotSet, // TODO: set this
		};
		
		notice::Entity::update(notice)
			.exec(&self.connection).await
			.map(|notice| Notice {
				id: NoticeSpecifier(notice.id),
				title: notice.title,
				content: notice.content,
				created_at: notice.created_at as _,
				expires_at: notice.expires_at.map(|v| v as _),
				author: None, // TODO: when this is set, retrieve it
			})
			.map(Reference::from)
			.map_err(DatabaseError::from)
	}

	// returns Ok(true) if the item was deleted or Ok(false) if it didn't exist
	pub async fn delete_notice(
		&self,
		notice: &NoticeSpecifier,
	) -> DbResult<bool> {
		let delete = notice::Entity::delete_by_id(notice.0)
			.exec(&self.connection).await?;
		
		Ok(delete.rows_affected > 0)
	}
}
