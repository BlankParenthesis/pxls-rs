use std::fmt;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use warp::http::{StatusCode, Uri};
use warp::{Filter, Reply, Rejection};

use crate::config::CONFIG;
use crate::filter::response::paginated_list::{
	PaginationOptions,
	PageToken,
};
use crate::filter::header::authorization::{self, Bearer};
use crate::filter::response::reference::Reference;
use crate::filter::resource::database;
use crate::permissions::Permission;
use crate::database::{UsersDatabase, UsersConnection, BoardsDatabase, BoardsConnection, BoardsDatabaseError, User};
use crate::routes::core::{EventPacket, Connections};

#[derive(Debug, Deserialize, Serialize, Clone, Copy)]
#[serde(rename_all = "UPPERCASE")]
pub enum ReportStatus {
	Opened,
	Closed,
}

#[derive(Serialize, Debug, Clone, Copy)]
pub enum ArtifactType {
	Board,
	Placement,
	BoardNotice,
	Notice,
	Faction,
	FactionMemeber,
	User,
	Role,
	Report,
}

#[derive(Serialize, Debug, Clone)]
pub struct Artifact {
	pub reference: Reference<()>,
	pub r#type: ArtifactType,
	pub timestamp: u64,
}

pub enum ArtifactParseError {
	Type,
	Host,
	Uri(warp::http::uri::InvalidUri),
}

impl Artifact {
	pub fn parse(uri: &str, timestamp: u64) -> Result<Self, ArtifactParseError> {
		let uri = uri.parse::<Uri>().map_err(ArtifactParseError::Uri)?;
		let path = uri.path()
			.split('/')
			.filter(|s| !s.is_empty())
			.collect::<Vec<_>>();

		let artifact_type = match &path[..] {
			["boards", id] => id.parse::<u32>()
				.map(|_| ArtifactType::Board)
				.map_err(|_| ArtifactParseError::Type),
			["boards", id, "pixels", id2] => id.parse::<u32>()
				.and_then(|_| id2.parse::<u64>())
				.map(|_| ArtifactType::Placement)
				.map_err(|_| ArtifactParseError::Type),
			["boards", id, "notice", id2] => id.parse::<u32>()
				.and_then(|_| id2.parse::<u32>())
				.map(|_| ArtifactType::BoardNotice)
				.map_err(|_| ArtifactParseError::Type),
			["notice", id] => id.parse::<u32>()
				.map(|_| ArtifactType::Notice)
				.map_err(|_| ArtifactParseError::Type),
			["faction", id] => id.parse::<u32>()
				.map(|_| ArtifactType::Faction)
				.map_err(|_| ArtifactParseError::Type),
			["faction", id, "member", id2] => id.parse::<u32>()
				.and_then(|_| id2.parse::<u32>())
				.map(|_| ArtifactType::Faction)
				.map_err(|_| ArtifactParseError::Type),
			["user", _id] => Ok(ArtifactType::User),
			["role", _id] => Ok(ArtifactType::Role),
			["report", id] => id.parse::<u32>()
				.map(|_| ArtifactType::Report)
				.map_err(|_| ArtifactParseError::Type),
			_ => Err(ArtifactParseError::Type),
		}?;

		Ok(Self {
			reference: Reference::new(uri, ()),
			r#type: artifact_type,
			timestamp,
		})
	}
}

#[serde_with::skip_serializing_none]
#[derive(Serialize, Debug, Clone)]
pub struct Report {
	pub status: ReportStatus,
	pub reason: String,
	pub artifacts: Vec<Artifact>,
	pub reporter: Option<Reference<User>>,
	pub timestamp: u64,
}

impl Report {
	pub fn uri(id: i32) -> Uri {
		format!("/reports/{}", id).parse::<Uri>().unwrap()
	}
}

#[derive(Default, Deserialize)]
pub struct ReportPageToken(pub usize);

impl PageToken for ReportPageToken {}

impl fmt::Display for ReportPageToken {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{}", self.0)
	}
}

#[derive(Deserialize, Debug)]
pub struct ReportFilter {
	// TODO: artifacts, reporter
	pub status: Option<ReportStatus>,
	pub reason: Option<String>,
}

pub fn list(
	boards_db: Arc<BoardsDatabase>,
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("reports")
		.and(warp::path::end())
		.and(warp::get())
		.and(warp::query())
		.and(warp::query())
		.and(authorization::authorized(users_db, Permission::ReportsList.into()))
		.and(database::connection(boards_db))
		.then(move |pagination: PaginationOptions<ReportPageToken>, filter: ReportFilter, _, mut users_connection: UsersConnection, boards_connection: BoardsConnection| async move {
			let page = pagination.page;
			let limit = pagination.limit
				.unwrap_or(CONFIG.default_page_item_limit)
				.clamp(1, CONFIG.max_page_item_limit);

			let page = boards_connection.list_reports(
				page,
				limit,
				filter,
				None,
				&mut users_connection,
			).await.map_err(Reply::into_response)?;
			
			Ok::<_, warp::reply::Response>(warp::reply::json(&page))
		})
}

pub fn owned(
	boards_db: Arc<BoardsDatabase>,
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("reports")
		.and(warp::path("owned"))
		.and(warp::path::end())
		.and(warp::get())
		.and(warp::query())
		.and(warp::query())
		.and(authorization::authorized(users_db, Permission::ReportsOwnedList.into()))
		.and(database::connection(boards_db))
		.then(move |pagination: PaginationOptions<ReportPageToken>, filter: ReportFilter, user: Option<Bearer>, mut users_connection: UsersConnection, boards_connection: BoardsConnection| async move {
			let page = pagination.page;
			let limit = pagination.limit
				.unwrap_or(CONFIG.default_page_item_limit)
				.clamp(1, CONFIG.max_page_item_limit);

			let page = boards_connection.list_reports(
				page,
				limit,
				filter,
				Some(user.map(|u| u.id)),
				&mut users_connection,
			).await.map_err(Reply::into_response)?;
			
			Ok::<_, warp::reply::Response>(warp::reply::json(&page))
		})
}

pub fn get(
	boards_db: Arc<BoardsDatabase>,
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("reports")
		.and(warp::path::param())
		.and(warp::path::end())
		.and(warp::get())
		.and(authorization::permissions(users_db))
		.and(database::connection(boards_db))
		.then(move |id: usize, user_permissions, bearer: Option<Bearer>, mut users_connection, boards_connection: BoardsConnection| async move {
			let report = boards_connection.get_report(id, &mut users_connection).await
				.map_err(Reply::into_response)?
				.ok_or(StatusCode::NOT_FOUND)
				.map_err(Reply::into_response)?;

			let report_owner = report.reporter.clone().map(|r| r.uri);
			let user = bearer.map(|b| User::uri(&b.id));
			let is_current_user = user == report_owner;

			let check = if is_current_user {
				authorization::has_permissions_current
			} else {
				authorization::has_permissions
			};

			if check(user_permissions, Permission::ReportsGet.into()) {
				Ok::<_, warp::reply::Response>(warp::reply::json(&report))
			} else {
				Err(StatusCode::FORBIDDEN.into_response())
			}
		})
}

#[derive(Debug, Deserialize)]
pub struct ArtifactPost {
	pub location: String,
	pub timestamp: u64,
}

#[derive(Debug, Deserialize)]
struct ReportPost {
	reason: String,
	artifacts: Vec<ArtifactPost>,
}

pub fn post(
	events: Arc<RwLock<Connections>>,
	boards_db: Arc<BoardsDatabase>,
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("reports")
		.and(warp::path::end())
		.and(warp::post())
		.and(warp::body::json())
		.and(authorization::authorized(users_db, Permission::ReportsPost.into()))
		.and(database::connection(boards_db))
		.then(move |report: ReportPost, user: Option<Bearer>, mut users_connection: UsersConnection, boards_connection: BoardsConnection| {
			let events = Arc::clone(&events);
			async move {
				let report = boards_connection.create_report(
					report.reason,
					user.map(|u| u.id),
					report.artifacts.into_iter()
						.map(|a| Artifact::parse(&a.location, a.timestamp))
						.collect::<Result<_, _>>()
						.map_err(|_| StatusCode::UNPROCESSABLE_ENTITY.into_response())?,
					&mut users_connection,
				).await.map_err(Reply::into_response)?;

				let reporter = report.view.reporter.as_ref().map(|r| {
					// get user id
					// TODO: this is an awful hack, make it nicer
					r.uri.path().split_once('/').unwrap().1.to_owned()
				});

				let packet = EventPacket::ReportCreated {
					report: report.clone(),
					reporter,
				};
				let events = events.write().await;
				events.send(&packet).await;
				
				Ok::<_, warp::reply::Response>(report.created())
			}
		})
}

#[derive(Debug, Deserialize)]
struct ReportPatch {
	status: Option<ReportStatus>,
	reason: Option<String>,
	artifacts: Option<Vec<ArtifactPost>>,
}

pub fn patch(
	events: Arc<RwLock<Connections>>,
	boards_db: Arc<BoardsDatabase>,
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("reports")
		.and(warp::path::param())
		.and(warp::path::end())
		.and(warp::patch())
		.and(warp::body::json())
		.and(authorization::authorized(users_db, Permission::ReportsGet | Permission::ReportsPatch))
		.and(database::connection(boards_db))
		.then(move |id: usize, report: ReportPatch, _, mut users_connection: UsersConnection, boards_connection: BoardsConnection| {
			let events = Arc::clone(&events);

			async move {
				let artifacts = match report.artifacts {
					Some(artifacts) => {
						Some(artifacts.into_iter()
							.map(|a| Artifact::parse(&a.location, a.timestamp))
							.collect::<Result<_, _>>()
							.map_err(|_| StatusCode::UNPROCESSABLE_ENTITY.into_response())?)
					},
					None => None,
				};

				let report = boards_connection.edit_report(
					id,
					report.status,
					report.reason,
					artifacts,
					&mut users_connection,
				).await.map_err(Reply::into_response)?;

				let reporter = report.view.reporter.as_ref().map(|r| {
					// get user id
					// TODO: this is an awful hack, make it nicer
					r.uri.path().split_once('/').unwrap().1.to_owned()
				});

				let packet = EventPacket::ReportUpdated {
					report: report.clone(),
					reporter,
				};
				let events = events.write().await;
				events.send(&packet).await;
				
				Ok::<_, warp::reply::Response>(warp::reply::json(&report))
			}
		})
}

pub fn delete(
	events: Arc<RwLock<Connections>>,
	boards_db: Arc<BoardsDatabase>,
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("reports")
		.and(warp::path::param())
		.and(warp::path::end())
		.and(warp::delete())
		.and(authorization::authorized(users_db, Permission::ReportsGet | Permission::ReportsDelete))
		.and(database::connection(boards_db))
		.then(move |id: usize, _: Option<Bearer>, _: UsersConnection, boards_connection: BoardsConnection| {
			let events = Arc::clone(&events);
			async move {				
				if let Some(user) = boards_connection.delete_report(id).await? {
					let packet = EventPacket::ReportDeleted {
						report: format!("/reports/{}", id)
							.parse::<Uri>().unwrap(),
						reporter: user,
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

pub fn history(
	boards_db: Arc<BoardsDatabase>,
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("reports")
		.and(warp::path::param())
		.and(warp::path("history"))
		.and(warp::path::end())
		.and(warp::get())
		.and(warp::query())
		.and(warp::query())
		.and(authorization::authorized(users_db, Permission::ReportsList.into()))
		.and(database::connection(boards_db))
		.then(move |id: usize, pagination: PaginationOptions<ReportPageToken>, filter: ReportFilter, _, mut users_connection: UsersConnection, boards_connection: BoardsConnection| async move {
			let page = pagination.page;
			let limit = pagination.limit
				.unwrap_or(CONFIG.default_page_item_limit)
				.clamp(1, CONFIG.max_page_item_limit);

			let page = boards_connection.list_report_history(
				id,
				page,
				limit,
				filter,
				&mut users_connection,
			).await.map_err(Reply::into_response)?;
			
			Ok::<_, warp::reply::Response>(warp::reply::json(&page))
		})
}
