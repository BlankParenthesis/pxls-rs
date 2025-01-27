use std::fmt;
use std::sync::Arc;

use serde::de::Visitor;
use serde::{Deserialize, Serialize, de, Deserializer};
use warp::http::{StatusCode, Uri};
use warp::{Filter, Reply, Rejection};

use crate::config::CONFIG;
use crate::BoardDataMap;
use crate::filter::resource::filter::FilterRange;
use crate::filter::response::paginated_list::{
	PaginationOptions,
	PageToken,
};
use crate::filter::header::authorization::{self, Bearer};
use crate::filter::response::reference::Reference;
use crate::filter::resource::database;
use crate::permissions::Permission;
use crate::database::{UsersDatabase, UsersConnection, BoardsDatabase, BoardsConnection, User};

#[serde_with::skip_serializing_none]
#[derive(Serialize, Debug, Clone)]
pub struct BoardsNotice {
	pub title: String,
	pub content: String,
	pub created_at: u64,
	pub expires_at: Option<u64>,
	pub author: Option<Reference<User>>,
}

impl BoardsNotice {
	pub fn uri(board_id: i32, notice_id: i32) -> Uri {
		format!("/board/{}/notices/{}", board_id, notice_id)
			.parse::<Uri>()
			.unwrap()
	}
}
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
	boards_db: Arc<BoardsDatabase>,
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("boards")
		.and(warp::path::param())
		.and(warp::path("notices"))
		.and(warp::path::end())
		.and(warp::get())
		.and(warp::query())
		.and(warp::query())
		.and(authorization::authorized(users_db, Permission::NoticesList.into()))
		.and(database::connection(boards_db))
		.then(move |board: usize, pagination: PaginationOptions<BoardsNoticePageToken>, filter: BoardNoticeFilter, _, mut users_connection: UsersConnection, boards_connection: BoardsConnection| {
			let boards = Arc::clone(&boards);
			async move {
				let boards = boards.read().await;
				if !boards.contains_key(&board) {
					return Err(StatusCode::NOT_FOUND.into_response());
				}

				let page = pagination.page;
				let limit = pagination.limit
					.unwrap_or(CONFIG.default_page_item_limit)
					.clamp(1, CONFIG.max_page_item_limit);

				let page = boards_connection.list_board_notices(
					board as i32,
					page,
					limit,
					filter,
					&mut users_connection,
				).await.map_err(Reply::into_response)?;

				Ok::<_, warp::reply::Response>(warp::reply::json(&page))
			}
		})
}

pub fn get(
	boards_db: Arc<BoardsDatabase>,
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("boards")
		.and(warp::path::param())
		.and(warp::path("notices"))
		.and(warp::path::param())
		.and(warp::path::end())
		.and(warp::get())
		.and(authorization::authorized(users_db, Permission::NoticesGet.into()))
		.and(database::connection(boards_db))
		.then(move |board: usize, id: usize, _: Option<Bearer>, mut users_connection: UsersConnection, boards_connection: BoardsConnection| async move {
			boards_connection.get_board_notice(
				board as i32,
				id,
				&mut users_connection,
			).await
				.map_err(Reply::into_response)?
				.ok_or(StatusCode::NOT_FOUND)
				.map_err(Reply::into_response)
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
	boards_db: Arc<BoardsDatabase>,
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("boards")
		.and(warp::path::param())
		.and(warp::path("notices"))
		.and(warp::path::end())
		.and(warp::post())
		.and(warp::body::json())
		.and(authorization::authorized(users_db, Permission::NoticesGet | Permission::NoticesPost))
		.and(database::connection(boards_db))
		.then(move |board: usize, notice: NoticePost, user: Option<Bearer>, mut users_connection: UsersConnection, boards_connection: BoardsConnection| {
			let boards = Arc::clone(&boards);
			async move {
				let boards = boards.read().await;
				let board = boards.get(&board)
					.ok_or(StatusCode::NOT_FOUND)
					.map_err(Reply::into_response)?;
				let board = board.read().await;
				let board = board.as_ref().expect("board went missing");

				// TODO: author
				board.create_notice(
					notice.title,
					notice.content,
					notice.expires_at,
					&boards_connection,
					&mut users_connection
				).await
					.map(|notice| notice.created())
					.map_err(Reply::into_response)
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
	boards_db: Arc<BoardsDatabase>,
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("boards")
		.and(warp::path::param())
		.and(warp::path("notices"))
		.and(warp::path::param())
		.and(warp::path::end())
		.and(warp::patch())
		.and(warp::body::json())
		.and(authorization::authorized(users_db, Permission::NoticesGet | Permission::NoticesPatch))
		.and(database::connection(boards_db))
		.then(move |board: usize, id: usize, notice: NoticePatch, user: Option<Bearer>, mut users_connection: UsersConnection, boards_connection: BoardsConnection| {
			let boards = Arc::clone(&boards);
			async move {
				let boards = boards.read().await;
				let board = boards.get(&board)
					.ok_or(StatusCode::NOT_FOUND)
					.map_err(Reply::into_response)?;
				let board = board.read().await;
				let board = board.as_ref().expect("board went missing");
				
				board.edit_notice(
					id,
					notice.title,
					notice.content,
					notice.expires_at,
					&boards_connection,
					&mut users_connection
				).await
					.map(|notice| notice.created()) // TODO: is created correct?
					.map_err(Reply::into_response)
			}
		})
}

pub fn delete(
	boards: BoardDataMap,
	boards_db: Arc<BoardsDatabase>,
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("boards")
		.and(warp::path::param())
		.and(warp::path("notices"))
		.and(warp::path::param())
		.and(warp::path::end())
		.and(warp::delete())
		.and(authorization::authorized(users_db, Permission::NoticesGet | Permission::NoticesDelete))
		.and(database::connection(boards_db))
		.then(move |board: usize, id: usize, _: Option<Bearer>, _: UsersConnection, boards_connection: BoardsConnection| {
			let boards = Arc::clone(&boards);
			async move {
				let boards = boards.read().await;
				let board = boards.get(&board)
					.ok_or(StatusCode::NOT_FOUND)
					.map_err(Reply::into_response)?;
				let board = board.read().await;
				let board = board.as_ref().expect("board went missing");

				let was_deleted = board.delete_notice(id, &boards_connection).await
					.map_err(Reply::into_response)?;

				if was_deleted {
					Ok::<_, warp::reply::Response>(StatusCode::NO_CONTENT)
				} else {
					Ok(StatusCode::NOT_FOUND)
				}
			}
		})
}
