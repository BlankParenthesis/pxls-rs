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
use crate::filter::header::authorization;
use crate::filter::response::reference::Reference;
use crate::permissions::Permission;
use crate::database::{BoardsDatabase, BoardsConnection, User};
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
	db: Arc<BoardsDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("reports")
		.and(warp::path::end())
		.and(warp::get())
		.and(warp::query())
		.and(warp::query())
		.and(authorization::authorized(db, Permission::ReportsList.into()))
		.then(move |pagination: PaginationOptions<ReportPageToken>, filter: ReportFilter, _, boards_connection: BoardsConnection| async move {
			let page = pagination.page;
			let limit = pagination.limit
				.unwrap_or(CONFIG.default_page_item_limit)
				.clamp(1, CONFIG.max_page_item_limit);

			let page = boards_connection.list_reports(
				page,
				limit,
				filter,
				None,
			).await?;
			
			Ok::<_, StatusCode>(warp::reply::json(&page))
		})
}

pub fn owned(
	db: Arc<BoardsDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("reports")
		.and(warp::path("owned"))
		.and(warp::path::end())
		.and(warp::get())
		.and(warp::query())
		.and(warp::query())
		.and(authorization::authorized(db, Permission::ReportsOwnedList.into()))
		.then(move |pagination: PaginationOptions<ReportPageToken>, filter: ReportFilter, user: Option<User>, connection: BoardsConnection| async move {
			let page = pagination.page;
			let limit = pagination.limit
				.unwrap_or(CONFIG.default_page_item_limit)
				.clamp(1, CONFIG.max_page_item_limit);

			let page = connection.list_reports(
				page,
				limit,
				filter,
				Some(user.as_ref()),
			).await?;
			
			Ok::<_, StatusCode>(warp::reply::json(&page))
		})
}

pub fn get(
	db: Arc<BoardsDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("reports")
		.and(warp::path::param())
		.and(warp::path::end())
		.and(warp::get())
		.and(authorization::permissions(db))
		.then(move |id: usize, user_permissions, user: Option<User>, connection: BoardsConnection| async move {
			let report = connection.get_report(id).await?
				.ok_or(StatusCode::NOT_FOUND)?;

			let report_owner = report.reporter.as_ref().map(|r| r.view.specifier());
			let user = user.map(|u| u.specifier());
			let is_current_user = user == report_owner;

			let check = if is_current_user {
				authorization::has_permissions_current
			} else {
				authorization::has_permissions
			};

			if check(user_permissions, Permission::ReportsGet.into()) {
				Ok(warp::reply::json(&report))
			} else {
				Err(StatusCode::FORBIDDEN)
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
	db: Arc<BoardsDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("reports")
		.and(warp::path::end())
		.and(warp::post())
		.and(warp::body::json())
		.and(authorization::authorized(db, Permission::ReportsPost.into()))
		.then(move |report: ReportPost, user: Option<User>, connection: BoardsConnection| {
			let events = Arc::clone(&events);
			async move {
				let artifacts = report.artifacts.into_iter()
					.map(|a| Artifact::parse(&a.location, a.timestamp))
					.collect::<Result<_, _>>()
					.map_err(|_| StatusCode::UNPROCESSABLE_ENTITY)?;
				
				let report = connection.create_report(
					report.reason,
					user.as_ref(),
					artifacts,
				).await?;

				let reporter = report.view.reporter.as_ref().map(|r| {
					r.view.specifier()
				});

				let packet = EventPacket::ReportCreated {
					report: report.clone(),
					reporter,
				};
				let events = events.write().await;
				events.send(&packet).await;
				
				Ok::<_, StatusCode>(report.created())
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
	db: Arc<BoardsDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("reports")
		.and(warp::path::param())
		.and(warp::path::end())
		.and(warp::patch())
		.and(warp::body::json())
		.and(authorization::authorized(db, Permission::ReportsGet | Permission::ReportsPatch))
		.then(move |id: usize, report: ReportPatch, _, connection: BoardsConnection| {
			let events = Arc::clone(&events);

			async move {
				let artifacts = match report.artifacts {
					Some(artifacts) => {
						Some(artifacts.into_iter()
							.map(|a| Artifact::parse(&a.location, a.timestamp))
							.collect::<Result<_, _>>()
							.map_err(|_| StatusCode::UNPROCESSABLE_ENTITY)?)
					},
					None => None,
				};

				let report = connection.edit_report(
					id,
					report.status,
					report.reason,
					artifacts,
				).await?;

				let reporter = report.view.reporter.as_ref().map(|r| {
					r.view.specifier()
				});

				let packet = EventPacket::ReportUpdated {
					report: report.clone(),
					reporter,
				};
				let events = events.write().await;
				events.send(&packet).await;
				
				Ok::<_, StatusCode>(warp::reply::json(&report))
			}
		})
}

pub fn delete(
	events: Arc<RwLock<Connections>>,
	db: Arc<BoardsDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("reports")
		.and(warp::path::param())
		.and(warp::path::end())
		.and(warp::delete())
		.and(authorization::authorized(db, Permission::ReportsGet | Permission::ReportsDelete))
		.then(move |id: usize, _, connection: BoardsConnection| {
			let events = Arc::clone(&events);
			async move {				
				if let Some(user) = connection.delete_report(id).await? {
					let packet = EventPacket::ReportDeleted {
						report: format!("/reports/{}", id)
							.parse::<Uri>().unwrap(),
						reporter: user,
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

pub fn history(
	db: Arc<BoardsDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("reports")
		.and(warp::path::param())
		.and(warp::path("history"))
		.and(warp::path::end())
		.and(warp::get())
		.and(warp::query())
		.and(warp::query())
		.and(authorization::authorized(db, Permission::ReportsList.into()))
		.then(move |id: usize, pagination: PaginationOptions<ReportPageToken>, filter: ReportFilter, _, connection: BoardsConnection| async move {
			let page = pagination.page;
			let limit = pagination.limit
				.unwrap_or(CONFIG.default_page_item_limit)
				.clamp(1, CONFIG.max_page_item_limit);

			let page = connection.list_report_history(
				id,
				page,
				limit,
				filter,
			).await?;
			
			Ok::<_, StatusCode>(warp::reply::json(&page))
		})
}
