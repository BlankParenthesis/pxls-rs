use std::time::{SystemTime, UNIX_EPOCH};

use sea_orm::sea_query::{Expr, SimpleExpr};
use sea_orm::{ActiveValue::NotSet, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect, QueryTrait, Set, StreamTrait, TransactionTrait};
use serde::{Deserialize, Serialize};
use warp::http::Uri;

use crate::database::filter::FilterRange;
use crate::database::user::UserSpecifier;
use crate::database::{BoardSpecifier, DbInsertResult, InsertError};
use crate::filter::response::paginated_list::Page;
use crate::filter::response::reference::{Referencable, Reference};
use crate::routes::board_notices::boards::notices::{BoardsNoticePageToken, BoardNoticeFilter};

use super::entities::*;

use super::{Connection, DbResult, DatabaseError};
use super::user::User;
use super::specifier::{Specifier, PathPart, SpecifierParser, SpecfierParseError, Id, specifier_path};

#[derive(Debug, Eq, PartialEq, Clone, Copy, Hash)]
pub struct BoardNoticeListSpecifier {
	board: i32,
}

impl BoardNoticeListSpecifier {
	pub fn board(&self) -> BoardSpecifier {
		BoardSpecifier(self.board)
	}
	
	fn notice(&self, notice: i32) -> BoardNoticeSpecifier {
		BoardNoticeSpecifier { board: self.board, notice }
	}
}

impl Specifier for BoardNoticeListSpecifier {
	fn filter(&self) -> SimpleExpr {
		board_notice::Column::Board.eq(self.board)
	}
	
	fn parts() -> &'static [PathPart] {
		specifier_path!("boards", board, "notices")
	}
	
	fn from_ids(ids: &[&str]) -> Result<Self, SpecfierParseError> {
		let board = ids[0].parse()?;
		Ok(Self { board })
	}
	
	fn to_ids(&self) -> Box<[Id]> {
		Box::new([Id::I32(self.board)])
	}
}

impl From<BoardSpecifier> for BoardNoticeListSpecifier {
	fn from(board: BoardSpecifier) -> Self {
		BoardNoticeListSpecifier { board: board.0 }
	}
}

impl<'de> Deserialize<'de> for BoardNoticeListSpecifier {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where D: serde::Deserializer<'de> {
		deserializer.deserialize_str(SpecifierParser::new("A board notice list uri"))
	}
}

impl Serialize for BoardNoticeListSpecifier {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where S: serde::Serializer {
		serializer.serialize_str(self.to_uri().path())
	}
}

#[derive(Debug, Eq, PartialEq, Clone, Copy, Hash)]
pub struct BoardNoticeSpecifier {
	board: i32,
	notice: i32,
}

impl BoardNoticeSpecifier {
	pub fn board(&self) -> BoardSpecifier {
		BoardSpecifier(self.board)
	}
	
	pub fn list(&self) -> BoardNoticeListSpecifier {
		BoardNoticeListSpecifier { board: self.board }
	}
}

impl Specifier for BoardNoticeSpecifier {
	fn filter(&self) -> SimpleExpr {
		board_notice::Column::Id.eq(self.notice)
			.and(board_notice::Column::Board.eq(self.board))
	}
	
	fn parts() -> &'static [PathPart] {
		specifier_path!("boards", board, "notices", notice)
	}
	
	fn from_ids(ids: &[&str]) -> Result<Self, SpecfierParseError> {
		let board = ids[0].parse()?;
		let notice = ids[1].parse()?;
		Ok(Self { board, notice })
	}
	
	fn to_ids(&self) -> Box<[Id]> {
		Box::new([Id::I32(self.board), Id::I32(self.notice)])
	}
}

impl<'de> Deserialize<'de> for BoardNoticeSpecifier {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where D: serde::Deserializer<'de> {
		deserializer.deserialize_str(SpecifierParser::new("A board notice uri"))
	}
}

impl Serialize for BoardNoticeSpecifier {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where S: serde::Serializer {
		serializer.serialize_str(self.to_uri().path())
	}
}

// TODO: unused?
#[derive(Deserialize, Debug)]
pub struct BoardsNoticeFilter {
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
pub struct BoardsNotice {
	#[serde(skip_serializing)]
	id: BoardNoticeSpecifier,
	pub title: String,
	pub content: String,
	pub created_at: u64,
	pub expires_at: Option<u64>,
	pub author: Option<Reference<User>>,
}

impl Referencable for BoardsNotice {
	fn uri(&self) -> Uri {
		self.id.to_uri()
	}
}

impl<C: TransactionTrait + ConnectionTrait + StreamTrait> Connection<C> {

	pub async fn list_board_notices(
		&self,
		list: &BoardNoticeListSpecifier,
		token: BoardsNoticePageToken,
		limit: usize,
		filter: BoardNoticeFilter,
	) -> DbResult<Page<Reference<BoardsNotice>>> {
		let column_timestamp_id_pair = Expr::tuple([
			Expr::col(board_notice::Column::CreatedAt).into(),
			Expr::col(board_notice::Column::Id).into(),
		]);

		let value_timestamp_id_pair = Expr::tuple([
			(token.timestamp as i64).into(),
			(token.id as i32).into(),
		]);

		let notices = board_notice::Entity::find()
			.find_also_related(user::Entity)
			.filter(list.filter())
			.filter(Expr::gte(column_timestamp_id_pair.clone(), value_timestamp_id_pair))
			.apply_if(filter.author.as_ref(), |q, id| q.filter(board_notice::Column::Author.eq(id)))
			.apply_if(filter.content.as_ref(), |q, content| q.filter(board_notice::Column::Content.eq(content)))
			.apply_if(filter.title.as_ref(), |q, title| q.filter(board_notice::Column::Title.eq(title)))
			.apply_if(filter.created_at.start, |q, start| q.filter(board_notice::Column::CreatedAt.gte(start)))
			.apply_if(filter.created_at.end, |q, end| q.filter(board_notice::Column::CreatedAt.lte(end)))
			.apply_if(filter.expires_at.start, |q, start| q.filter(board_notice::Column::ExpiresAt.gte(start)))
			.apply_if(filter.expires_at.end, |q, end| q.filter(board_notice::Column::ExpiresAt.lte(end)))
			.order_by(column_timestamp_id_pair, sea_orm::Order::Asc)
			.limit(limit as u64 + 1)
			.all(&self.connection).await?;

		let next = notices.windows(2).nth(limit.saturating_sub(1))
			.map(|pair| &pair[0])
			.map(|(notice, _)| BoardsNoticePageToken {
				id: notice.id as _,
				timestamp: notice.created_at as _,
			})
			.map(|token| {
				let uri = list.to_uri();
				let path = uri.path();
				// TODO: filter
				format!("{path}?page={token}&limit={limit}").parse().unwrap()
			});
		
		let notices = notices.into_iter()
			.take(limit)
			.map(|(notice, author)| BoardsNotice {
				id: list.notice(notice.id),
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

	pub async fn get_board_notice(
		&self,
		notice: &BoardNoticeSpecifier,
	) -> DbResult<Option<BoardsNotice>> {
		let notice = board_notice::Entity::find()
			.find_also_related(user::Entity)
			.filter(notice.filter())
			.one(&self.connection).await?
			.map(|(model, author)| BoardsNotice {
				id: *notice,
				title: model.title,
				content: model.content,
				created_at: model.created_at as _,
				expires_at: model.expires_at.map(|v| v as _),
				author: author.map(User::from).map(Reference::from),
			});
		
		Ok(notice)
	}

	pub async fn create_board_notice(
		&self,
		list: BoardNoticeListSpecifier,
		title: String,
		content: String,
		expiry: Option<u64>,
		author: Option<&UserSpecifier>,
	) -> DbInsertResult<BoardsNotice> {
		let now = SystemTime::now()
			.duration_since(UNIX_EPOCH)
			.unwrap()
			.as_secs();
		
		let notice = board_notice::ActiveModel {
			id: NotSet,
			board: Set(list.board),
			title: Set(title),
			content: Set(content),
			created_at: Set(now as _),
			expires_at: Set(expiry.map(|v| v as _)),
			author: Set(author.map(|u| u.0)),
		};
		
		let transaction = self.begin().await?;
		
		let author = if let Some(author) = author {
			let user = self.get_user(author).await?
				.ok_or(InsertError::MissingDependency)?;
			Some(user)
		} else {
			None
		};

		let insert = board_notice::Entity::insert(notice)
			.exec_with_returning(&transaction.connection).await?;
		
		transaction.commit().await?;
		
		let notice = BoardsNotice {
			id: list.notice(insert.id),
			title: insert.title,
			content: insert.content,
			created_at: insert.created_at as _,
			expires_at: insert.expires_at.map(|v| v as _),
			author: author.map(Reference::from),
		};
		
		Ok(notice)
	}

	pub async fn edit_board_notice(
		&self,
		notice: &BoardNoticeSpecifier,
		title: Option<String>,
		content: Option<String>,
		expiry: Option<Option<u64>>,
	) -> DbResult<Reference<BoardsNotice>> {
		let model = board_notice::ActiveModel {
			board: NotSet,
			id: Set(notice.notice),
			title: title.map(Set).unwrap_or(NotSet),
			content: content.map(Set).unwrap_or(NotSet),
			created_at: NotSet,
			expires_at: expiry.map(|e| Set(e.map(|v| v as _))).unwrap_or(NotSet),
			author: NotSet, // TODO: set this
		};
		
		board_notice::Entity::update(model)
			.filter(notice.filter())
			.exec(&self.connection).await
			.map(|model| BoardsNotice {
				id: *notice,
				title: model.title,
				content: model.content,
				created_at: model.created_at as _,
				expires_at: model.expires_at.map(|v| v as _),
				author: None, // TODO: when this is set, it will have to be fetched
			})
			.map(Reference::from)
			.map_err(DatabaseError::from)
	}

	// returns Ok(true) if the item was deleted or Ok(false) if it didn't exist
	pub async fn delete_board_notice(
		&self,
		notice: &BoardNoticeSpecifier,
	) -> DbResult<bool> {
		let delete = board_notice::Entity::delete_many()
			.filter(notice.filter())
			.exec(&self.connection).await?;
		
		Ok(delete.rows_affected > 0)
	}
}
