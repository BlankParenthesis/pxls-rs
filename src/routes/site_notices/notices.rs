use std::fmt;
use std::sync::Arc;

use serde::de::Visitor;
use serde::{Deserialize, Serialize, de, Deserializer};
use tokio::sync::RwLock;
use warp::http::{StatusCode, Uri};
use warp::{Filter, Reply, Rejection};

use crate::config::CONFIG;
use crate::filter::resource::filter::FilterRange;
use crate::filter::response::paginated_list::{
	PaginationOptions,
	PageToken,
};
use crate::filter::header::authorization;
use crate::filter::response::reference::Reference;
use crate::permissions::Permission;
use crate::database::{BoardsDatabase, BoardsConnection, User};
use crate::routes::core::{EventPacket, Connections};


#[serde_with::skip_serializing_none]
#[derive(Serialize, Debug, Clone)]
pub struct Notice {
	pub title: String,
	pub content: String,
	pub created_at: u64,
	pub expires_at: Option<u64>,
	pub author: Option<Reference<User>>,
}

impl Notice {
	pub fn uri(id: i32) -> Uri {
		format!("/notices/{}", id).parse::<Uri>().unwrap()
	}
}

#[derive(Default)]
pub struct NoticePageToken {
	pub id: usize,
	pub timestamp: u64,
}

impl PageToken for NoticePageToken {}

impl fmt::Display for NoticePageToken {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{}_{}", self.id, self.timestamp)
	}
}

impl<'de> Deserialize<'de> for NoticePageToken {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where
		D: Deserializer<'de>,
	{
		struct PageVisitor;

		impl<'de> Visitor<'de> for PageVisitor {
			type Value = NoticePageToken;

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
						Ok(NoticePageToken {
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
pub struct NoticeFilter {
	pub title: Option<String>,
	pub content: Option<String>,
	#[serde(default)]
	pub created_at: FilterRange<u64>,
	#[serde(default)]
	pub expires_at: FilterRange<u64>, // TODO: explicit null?
	pub author: Option<String>, // TODO: uri inference
}

pub fn list(
	db: Arc<BoardsDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("notices")
		.and(warp::path::end())
		.and(warp::get())
		.and(warp::query())
		.and(warp::query())
		.and(authorization::authorized(db, Permission::NoticesList.into()))
		.then(move |pagination: PaginationOptions<NoticePageToken>, filter: NoticeFilter, _, connection: BoardsConnection| async move {
			let page = pagination.page;
			let limit = pagination.limit
				.unwrap_or(CONFIG.default_page_item_limit)
				.clamp(1, CONFIG.max_page_item_limit);

			let page = connection.list_notices(page, limit, filter).await?;
			
			Ok::<_, StatusCode>(warp::reply::json(&page))
		})
}

pub fn get(
	db: Arc<BoardsDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("notices")
		.and(warp::path::param())
		.and(warp::path::end())
		.and(warp::get())
		.and(authorization::authorized(db, Permission::NoticesGet.into()))
		.then(move |id: usize, _, connectioon: BoardsConnection| async move {
			let notice = connectioon.get_notice(id).await?
				.ok_or(StatusCode::NOT_FOUND)?;
			
			Ok::<_, StatusCode>(warp::reply::json(&notice))
		})
}


#[derive(Debug, Deserialize)]
struct NoticePost {
	title: String,
	content: String,
	expires_at: Option<u64>,
}

pub fn post(
	events: Arc<RwLock<Connections>>,
	db: Arc<BoardsDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("notices")
		.and(warp::path::end())
		.and(warp::post())
		.and(warp::body::json())
		.and(authorization::authorized(db, Permission::NoticesGet | Permission::NoticesPost))
		.then(move |notice: NoticePost, user: Option<User>, boards_connection: BoardsConnection| {
			let events = Arc::clone(&events);
			async move {
				// TODO: author (requires spec decision)
				
				let notice = boards_connection.create_notice(
					notice.title,
					notice.content,
					notice.expires_at,
				).await?;

				let packet = EventPacket::SiteNoticeCreated {
					notice: notice.clone(),
				};
				let events = events.write().await;
				events.send(&packet).await;
				
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
	events: Arc<RwLock<Connections>>,
	db: Arc<BoardsDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("notices")
		.and(warp::path::param())
		.and(warp::path::end())
		.and(warp::patch())
		.and(warp::body::json())
		.and(authorization::authorized(db, Permission::NoticesGet | Permission::NoticesPatch))
		.then(move |id: usize, notice: NoticePatch, user: Option<User>, connection: BoardsConnection| {
			let events = Arc::clone(&events);
			async move {
				// TODO: author

				let notice = connection.edit_notice(
					id,
					notice.title,
					notice.content,
					notice.expires_at,
				).await?;

				let packet = EventPacket::SiteNoticeUpdated {
					notice: notice.clone(),
				};
				let events = events.write().await;
				events.send(&packet).await;
				
				Ok::<_, StatusCode>(warp::reply::json(&notice))
			}
		})
}

pub fn delete(
	events: Arc<RwLock<Connections>>,
	boards_db: Arc<BoardsDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("notices")
		.and(warp::path::param())
		.and(warp::path::end())
		.and(warp::delete())
		.and(authorization::authorized(boards_db, Permission::NoticesGet | Permission::NoticesDelete))
		.then(move |id: usize, _, connection: BoardsConnection| {
			let events = Arc::clone(&events);
			async move {
				let was_deleted = connection.delete_notice(id).await?;

				if was_deleted {
					let packet = EventPacket::SiteNoticeDeleted {
						notice: format!("/notices/{}", id)
							.parse::<Uri>().unwrap()
					};
					let events = events.write().await;
					events.send(&packet).await;

					Ok::<_, StatusCode>(StatusCode::NO_CONTENT)
				} else {
					Ok::<_, StatusCode>(StatusCode::NOT_FOUND)
				}
			}
		})
}
