use std::fmt;
use std::sync::Arc;

use serde::de::Visitor;
use serde::{Deserialize, Serialize, de, Deserializer};
use warp::http::{StatusCode, Uri};
use warp::{Filter, Reply, Rejection};

use crate::BoardDataMap;
use crate::filter::response::paginated_list::{
	PaginationOptions,
	DEFAULT_PAGE_ITEM_LIMIT,
	MAX_PAGE_ITEM_LIMIT,
	PageToken
};
use crate::filter::header::authorization::{self, Bearer};
use crate::filter::response::reference::{self, Referenceable, Reference};
use crate::filter::resource::database;
use crate::permissions::Permission;
use crate::database::{UsersDatabase, UsersConnection, BoardsDatabase, BoardsConnection};

#[serde_with::skip_serializing_none]
#[derive(Serialize, Debug)]
pub struct BoardsNotice {
	#[serde(skip_serializing)]
	pub board: usize,
	#[serde(skip_serializing)]
	pub id: usize,
	pub title: String,
	pub content: String,
	pub created_at: u64,
	pub expires_at: Option<u64>,
	pub author: Option<String>,
}

impl From<&BoardsNotice> for Uri {
	fn from(notice: &BoardsNotice) -> Self {
		format!("/boards/{}/notices/{}", notice.board, notice.id)
			.parse::<Uri>()
			.unwrap()
	}
}

impl Referenceable for BoardsNotice {
	fn location(&self) -> Uri { Uri::from(self)}
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
		.and(authorization::authorized(users_db, Permission::NoticesList.into()))
		.and(database::connection(boards_db))
		.then(move |board: usize, pagination: PaginationOptions<BoardsNoticePageToken>, _, mut users_connection: UsersConnection, boards_connection: BoardsConnection| {
			let boards = Arc::clone(&boards);
			async move {
				let boards = boards.read().await;
				if !boards.contains_key(&board) {
					return Err(StatusCode::NOT_FOUND.into_response());
				}

				let page = pagination.page;
				let limit = pagination.limit
					.unwrap_or(DEFAULT_PAGE_ITEM_LIMIT)
					.clamp(1, MAX_PAGE_ITEM_LIMIT);

				// TODO: map author id to user

				boards_connection.list_board_notices(board as i32, page, limit).await
					.map(|page| warp::reply::json(&page.into_references()))
					.map_err(Reply::into_response)
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
		.then(move |board: usize, id: usize, _: Option<Bearer>, _: UsersConnection, boards_connection: BoardsConnection| async move {
			// TODO: map author id to user
			boards_connection.get_board_notice(board as i32, id).await
				.map_err(Reply::into_response)?
				.map(|placement| warp::reply::json(&placement))
				.ok_or(StatusCode::NOT_FOUND)
				.map_err(Reply::into_response)

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
		.then(move |board: usize, notice: NoticePost, user: Option<Bearer>, _: UsersConnection, boards_connection: BoardsConnection| {
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
				).await
					.map(|notice| reference::created(&notice))
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
		.then(move |board: usize, id: usize, notice: NoticePatch, user: Option<Bearer>, _: UsersConnection, boards_connection: BoardsConnection| {
			let boards = Arc::clone(&boards);
			async move {
				let boards = boards.read().await;
				let board = boards.get(&board)
					.ok_or(StatusCode::NOT_FOUND)
					.map_err(Reply::into_response)?;
				let board = board.read().await;
				let board = board.as_ref().expect("board went missing");
				// TODO: author

				board.edit_notice(
					id,
					notice.title,
					notice.content,
					notice.expires_at,
					&boards_connection,
				).await
					.map(|notice| warp::reply::json(&Reference::from(&notice)))
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