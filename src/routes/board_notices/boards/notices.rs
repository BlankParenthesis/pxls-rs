use std::fmt;
use std::sync::Arc;

use serde::de::Visitor;
use serde::{Deserialize, de, Deserializer};
use warp::http::StatusCode;
use warp::{Filter, Reply, Rejection};

use crate::config::CONFIG;
use crate::BoardDataMap;
use crate::filter::resource::filter::FilterRange;
use crate::filter::response::paginated_list::{
	PaginationOptions,
	PageToken,
};
use crate::filter::header::authorization;
use crate::permissions::Permission;
use crate::database::{BoardNoticeListSpecifier, BoardNoticeSpecifier, Database, DbConn, Specifier, User};

#[derive(Default)]
pub struct BoardsNoticePageToken {
	pub id: usize,
	pub timestamp: u64,
}

impl PageToken for BoardsNoticePageToken {}

impl fmt::Display for BoardsNoticePageToken {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{}_{}", self.id, self.timestamp)
	}
}

impl<'de> Deserialize<'de> for BoardsNoticePageToken {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where
		D: Deserializer<'de>,
	{
		struct PageVisitor;

		impl<'de> Visitor<'de> for PageVisitor {
			type Value = BoardsNoticePageToken;

			fn expecting(
				&self,
				formatter: &mut fmt::Formatter,
			) -> fmt::Result {
				formatter.write_str("a string of two integers, separated by an underscore")
			}

			fn visit_str<E>(
				self,
				value: &str,
			) -> Result<Self::Value, E>
			where
				E: de::Error,
			{
				value.split_once('_')
					.ok_or_else(|| E::custom("missing underscore"))
					.and_then(|(timestamp, id)| {
						Ok(BoardsNoticePageToken {
							id: id
								.parse()
								.map_err(|_| E::custom("id invalid"))?,
							timestamp: timestamp
								.parse()
								.map_err(|_| E::custom("timestamp invalid"))?,
						})
					})
			}
		}

		deserializer.deserialize_str(PageVisitor)
	}
}

#[derive(Deserialize, Debug)]
pub struct BoardNoticeFilter {
	pub title: Option<String>,
	pub content: Option<String>,
	#[serde(default)]
	pub created_at: FilterRange<u64>,
	#[serde(default)]
	pub expires_at: FilterRange<u64>, // TODO: explicit null?
	pub author: Option<String>, // TODO: uri inference
}

pub fn list(
	boards: BoardDataMap,
	db: Arc<Database>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	BoardNoticeListSpecifier::path()
		.and(warp::get())
		.and(warp::query())
		.and(warp::query())
		.and(authorization::authorized(db, Permission::NoticesList.into()))
		.then(move |list: BoardNoticeListSpecifier, pagination: PaginationOptions<BoardsNoticePageToken>, filter: BoardNoticeFilter, _, connection: DbConn| {
			let boards = Arc::clone(&boards);
			async move {
				let boards = boards.read().await;
				if !boards.contains_key(&list.board()) {
					return Err(StatusCode::NOT_FOUND);
				}

				let page = pagination.page;
				let limit = pagination.limit
					.unwrap_or(CONFIG.default_page_item_limit)
					.clamp(1, CONFIG.max_page_item_limit);

				let page = connection.list_board_notices(
					&list,
					page,
					limit,
					filter,
				).await?;

				Ok(warp::reply::json(&page))
			}
		})
}

pub fn get(
	db: Arc<Database>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	BoardNoticeSpecifier::path()
		.and(warp::get())
		.and(authorization::authorized(db, Permission::NoticesGet.into()))
		.then(move |notice, _, connection: DbConn| async move {
			connection.get_board_notice(&notice).await?
				.ok_or(StatusCode::NOT_FOUND)
				.map(|notice| warp::reply::json(&notice))
		})
}


#[derive(Debug, Deserialize)]
struct NoticePost {
	title: String,
	content: String,
	expires_at: Option<u64>,
}

pub fn post(
	boards: BoardDataMap,
	db: Arc<Database>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	BoardNoticeListSpecifier::path()
		.and(warp::post())
		.and(warp::body::json())
		.and(authorization::authorized(db, Permission::NoticesGet | Permission::NoticesPost))
		.then(move |list: BoardNoticeListSpecifier, post: NoticePost, user: Option<User>, connection: DbConn| {
			let boards = Arc::clone(&boards);
			async move {
				let boards = boards.read().await;
				let board = boards.get(&list.board())
					.ok_or(StatusCode::NOT_FOUND)?;
				let board = board.read().await;
				let board = board.as_ref().expect("board went missing");

				let notice = board.create_notice(
					post.title,
					post.content,
					post.expires_at,
					user.as_ref(),
					&connection,
				).await?;
				
				Ok::<_, StatusCode>(notice.created())
			}
		})
}

#[derive(Debug, Deserialize)]
struct NoticePatch {
	title: Option<String>,
	content: Option<String>,
	#[serde(default, with = "serde_with::rust::double_option")]
	expires_at: Option<Option<u64>>,
}

pub fn patch(
	boards: BoardDataMap,
	db: Arc<Database>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	BoardNoticeSpecifier::path()
		.and(warp::patch())
		.and(warp::body::json())
		.and(authorization::authorized(db, Permission::NoticesGet | Permission::NoticesPatch))
		.then(move |notice: BoardNoticeSpecifier, patch: NoticePatch, _, connection: DbConn| {
			let boards = Arc::clone(&boards);
			async move {
				let boards = boards.read().await;
				let board = boards.get(&notice.board())
					.ok_or(StatusCode::NOT_FOUND)?;
				let board = board.read().await;
				let board = board.as_ref().expect("board went missing");
				
				let notice = board.edit_notice(
					&notice,
					patch.title,
					patch.content,
					patch.expires_at,
					&connection,
				).await?;
				
				Ok::<_, StatusCode>(notice.created()) // TODO: is created correct?
			}
		})
}

pub fn delete(
	boards: BoardDataMap,
	db: Arc<Database>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	BoardNoticeSpecifier::path()
		.and(warp::delete())
		.and(authorization::authorized(db, Permission::NoticesGet | Permission::NoticesDelete))
		.then(move |notice: BoardNoticeSpecifier, _, connection: DbConn| {
			let boards = Arc::clone(&boards);
			async move {
				let boards = boards.read().await;
				let board = boards.get(&notice.board())
					.ok_or(StatusCode::NOT_FOUND)?;
				let board = board.read().await;
				let board = board.as_ref().expect("board went missing");

				let was_deleted = board.delete_notice(notice, &connection).await?;

				if was_deleted {
					Ok::<_, StatusCode>(StatusCode::NO_CONTENT)
				} else {
					Ok(StatusCode::NOT_FOUND)
				}
			}
		})
}
