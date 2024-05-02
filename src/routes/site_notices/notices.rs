use std::fmt;
use std::sync::Arc;

use serde::de::Visitor;
use serde::{Deserialize, Serialize, de, Deserializer};
use tokio::sync::RwLock;
use warp::http::{StatusCode, Uri};
use warp::{Filter, Reply, Rejection};

use crate::filter::response::paginated_list::{
	PaginationOptions,
	DEFAULT_PAGE_ITEM_LIMIT,
	MAX_PAGE_ITEM_LIMIT,
	PageToken, Page
};
use crate::filter::header::authorization::{self, Bearer};
use crate::filter::response::reference::{self, Referenceable, Reference};
use crate::filter::resource::database;
use crate::permissions::Permission;
use crate::database::{UsersDatabase, UsersConnection, BoardsDatabase, BoardsConnection, BoardsDatabaseError, UsersDatabaseError, User};
use crate::routes::core::{EventPacket, Connections};


#[derive(Debug)]
pub struct Notice {
	pub id: usize,
	pub title: String,
	pub content: String,
	pub created_at: u64,
	pub expires_at: Option<u64>,
	pub author: Option<String>,
}

impl Notice {
	async fn prepare(
		self,
		connection: &mut UsersConnection,
	) -> Result<PreparedNotice, UsersDatabaseError> {
		let Self { id, title, content, created_at, expires_at, .. } = self;
		let author = if let Some(id) = self.author {
			let user = connection.get_user(&id).await?;
			Some(Reference::from(user))
		} else {
			None
		};

		Ok(PreparedNotice {
			id,
			title,
			content,
			created_at,
			expires_at,
			author,
		})
	} 
}

#[serde_with::skip_serializing_none]
#[derive(Serialize, Debug)]
pub struct PreparedNotice {
	#[serde(skip_serializing)]
	pub id: usize,
	pub title: String,
	pub content: String,
	pub created_at: u64,
	pub expires_at: Option<u64>,
	pub author: Option<Reference<User>>,
}

impl From<&PreparedNotice> for Uri {
	fn from(notice: &PreparedNotice) -> Self {
		format!("/notices/{}", notice.id)
			.parse::<Uri>()
			.unwrap()
	}
}

impl Referenceable for PreparedNotice {
	fn location(&self) -> Uri { Uri::from(self)}
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


pub fn list(
	boards_db: Arc<BoardsDatabase>,
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("notices")
		.and(warp::path::end())
		.and(warp::get())
		.and(warp::query())
		.and(authorization::authorized(users_db, Permission::NoticesList.into()))
		.and(database::connection(boards_db))
		.then(move |pagination: PaginationOptions<NoticePageToken>, _, mut users_connection: UsersConnection, boards_connection: BoardsConnection| async move {
			let page = pagination.page;
			let limit = pagination.limit
				.unwrap_or(DEFAULT_PAGE_ITEM_LIMIT)
				.clamp(1, MAX_PAGE_ITEM_LIMIT);

			let page = boards_connection.list_notices(page, limit).await
				.map_err(Reply::into_response)?;
			
			let mut items = Vec::with_capacity(page.items.len());
			
			for item in page.items {
				let notice = item.prepare(&mut users_connection).await
					.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())?;
				items.push(notice);
			}

			let page = Page {
				next: page.next,
				previous: page.previous,
				items,
			};

			Ok::<_, warp::reply::Response>(warp::reply::json(&page.into_references()))
		})
}

pub fn get(
	boards_db: Arc<BoardsDatabase>,
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("notices")
		.and(warp::path::param())
		.and(warp::path::end())
		.and(warp::get())
		.and(authorization::authorized(users_db, Permission::NoticesGet.into()))
		.and(database::connection(boards_db))
		.then(move |id: usize, _: Option<Bearer>, mut users_connection: UsersConnection, boards_connection: BoardsConnection| async move {
			let notice = boards_connection.get_notice(id).await
				.map_err(Reply::into_response)?
				.ok_or(StatusCode::NOT_FOUND)
				.map_err(Reply::into_response)?;
			notice.prepare(&mut users_connection).await
				.map(|notice| warp::reply::json(&notice))
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
	events: Arc<RwLock<Connections>>,
	boards_db: Arc<BoardsDatabase>,
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("notices")
		.and(warp::path::end())
		.and(warp::post())
		.and(warp::body::json())
		.and(authorization::authorized(users_db, Permission::NoticesGet | Permission::NoticesPost))
		.and(database::connection(boards_db))
		.then(move |notice: NoticePost, user: Option<Bearer>, mut users_connection: UsersConnection, boards_connection: BoardsConnection| {
			let events = Arc::clone(&events);
			async move {
				// TODO: author (requires spec decision)
				
				let notice = boards_connection.create_notice(
					notice.title,
					notice.content,
					notice.expires_at,
				).await
					.map_err(Reply::into_response)?
					.prepare(&mut users_connection).await
					.map_err(Reply::into_response)?;

				let packet = EventPacket::SiteNoticeCreated {
					notice: Reference::from(&notice),
				};
				let events = events.write().await;
				events.send(&packet).await;
				
				Ok::<_, warp::reply::Response>(reference::created(&notice))
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
	boards_db: Arc<BoardsDatabase>,
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("notices")
		.and(warp::path::param())
		.and(warp::path::end())
		.and(warp::patch())
		.and(warp::body::json())
		.and(authorization::authorized(users_db, Permission::NoticesGet | Permission::NoticesPatch))
		.and(database::connection(boards_db))
		.then(move |id: usize, notice: NoticePatch, user: Option<Bearer>, mut users_connection: UsersConnection, boards_connection: BoardsConnection| {
			let events = Arc::clone(&events);
			async move {
				// TODO: author

				let notice = boards_connection.edit_notice(
					id,
					notice.title,
					notice.content,
					notice.expires_at,
				).await
					.map_err(Reply::into_response)?
					.prepare(&mut users_connection).await
					.map_err(Reply::into_response)?;

				let reference = Reference::from(&notice);
				
				let packet = EventPacket::SiteNoticeUpdated {
					notice: reference.clone(),
				};
				let events = events.write().await;
				events.send(&packet).await;
				
				Ok::<_, warp::reply::Response>(warp::reply::json(&reference))
			}
		})
}

pub fn delete(
	events: Arc<RwLock<Connections>>,
	boards_db: Arc<BoardsDatabase>,
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("notices")
		.and(warp::path::param())
		.and(warp::path::end())
		.and(warp::delete())
		.and(authorization::authorized(users_db, Permission::NoticesGet | Permission::NoticesDelete))
		.and(database::connection(boards_db))
		.then(move |id: usize, _: Option<Bearer>, _: UsersConnection, boards_connection: BoardsConnection| {
			let events = Arc::clone(&events);
			async move {
				let was_deleted = boards_connection.delete_notice(id).await?;

				if was_deleted {
					let packet = EventPacket::SiteNoticeDeleted {
						notice: format!("/notices/{}", id)
							.parse::<Uri>().unwrap()
					};
					let events = events.write().await;
					events.send(&packet).await;

					Ok::<_, BoardsDatabaseError>(StatusCode::NO_CONTENT)
				} else {
					Ok::<_, BoardsDatabaseError>(StatusCode::NOT_FOUND)
				}
			}
		})
}