use std::sync::Arc;

use serde::Deserialize;
use tokio::sync::RwLock;
use warp::http::StatusCode;
use warp::{Filter, Reply, Rejection};

use crate::config::CONFIG;
use crate::filter::response::paginated_list::PaginationOptions;
use crate::filter::header::authorization;
use crate::filter::response::reference::Reference;
use crate::permissions::Permission;
use crate::database::{Artifact, Database, DbConn, ReportFilter, ReportHistorySpecifier, ReportPageToken, ReportSpecifier, ReportStatus, Specifier, User};
use crate::routes::core::{EventPacket, Connections};

pub fn list(
	db: Arc<Database>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("reports")
		.and(warp::path::end())
		.and(warp::get())
		.and(warp::query())
		.and(warp::query())
		.and(authorization::authorized(db, Permission::ReportsList.into()))
		.then(move |pagination: PaginationOptions<ReportPageToken>, filter: ReportFilter, _, connection: DbConn| async move {
			let page = pagination.page;
			let limit = pagination.limit
				.unwrap_or(CONFIG.default_page_item_limit)
				.clamp(1, CONFIG.max_page_item_limit);

			let page = connection.list_reports(
				page,
				limit,
				filter,
				None,
			).await?;
			
			Ok::<_, StatusCode>(warp::reply::json(&page))
		})
}

pub fn owned(
	db: Arc<Database>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("reports")
		.and(warp::path("owned"))
		.and(warp::path::end())
		.and(warp::get())
		.and(warp::query())
		.and(warp::query())
		.and(authorization::authorized(db, Permission::ReportsOwnedList.into()))
		.then(move |pagination: PaginationOptions<ReportPageToken>, filter: ReportFilter, user: Option<User>, connection: DbConn| async move {
			let page = pagination.page;
			let limit = pagination.limit
				.unwrap_or(CONFIG.default_page_item_limit)
				.clamp(1, CONFIG.max_page_item_limit);

			let page = connection.list_reports(
				page,
				limit,
				filter,
				Some(user.map(|u| *u.specifier()).as_ref()),
			).await?;
			
			Ok::<_, StatusCode>(warp::reply::json(&page))
		})
}

pub fn get(
	db: Arc<Database>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	ReportSpecifier::path()
		.and(warp::get())
		.and(authorization::permissions(db))
		.then(move |report: ReportSpecifier, user_permissions, user: Option<User>, connection: DbConn| async move {
			let report = connection.get_report(&report).await?
				.ok_or(StatusCode::NOT_FOUND)?;

			let report_owner = report.reporter.as_ref().map(|r| r.view.specifier());
			let user = user.map(|u| *u.specifier());
			let is_current_user = user.as_ref() == report_owner;

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
	db: Arc<Database>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("reports")
		.and(warp::path::end())
		.and(warp::post())
		.and(warp::body::json())
		.and(authorization::authorized(db, Permission::ReportsPost.into()))
		.then(move |report: ReportPost, user: Option<User>, connection: DbConn| {
			let events = Arc::clone(&events);
			async move {
				let artifacts = report.artifacts.into_iter()
					.map(|a| Artifact::parse(&a.location, a.timestamp))
					.collect::<Result<_, _>>()
					.map_err(|_| StatusCode::UNPROCESSABLE_ENTITY)?;
				
				let report = connection.create_report(
					report.reason,
					user.map(|u| *u.specifier()).as_ref(),
					artifacts,
				).await?;

				let reporter = report.reporter.as_ref().map(|r| {
					*r.view.specifier()
				});
				
				let report = Reference::from(report);

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
	db: Arc<Database>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	ReportSpecifier::path()
		.and(warp::patch())
		.and(warp::body::json())
		.and(authorization::authorized(db, Permission::ReportsGet | Permission::ReportsPatch))
		.then(move |report: ReportSpecifier, patch: ReportPatch, _, connection: DbConn| {
			let events = Arc::clone(&events);

			async move {
				let artifacts = match patch.artifacts {
					Some(artifacts) => {
						Some(artifacts.into_iter()
							.map(|a| Artifact::parse(&a.location, a.timestamp))
							.collect::<Result<_, _>>()
							.map_err(|_| StatusCode::UNPROCESSABLE_ENTITY)?)
					},
					None => None,
				};

				let report = connection.edit_report(
					&report,
					patch.status,
					patch.reason,
					artifacts,
				).await?;

				let reporter = report.reporter.as_ref().map(|r| {
					*r.view.specifier()
				});
				
				let report = Reference::from(report);

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
	db: Arc<Database>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	ReportSpecifier::path()
		.and(warp::delete())
		.and(authorization::authorized(db, Permission::ReportsGet | Permission::ReportsDelete))
		.then(move |report: ReportSpecifier, _, connection: DbConn| {
			let events = Arc::clone(&events);
			async move {				
				if let Some(user) = connection.delete_report(report).await? {
					let packet = EventPacket::ReportDeleted {
						report,
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
	db: Arc<Database>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	ReportHistorySpecifier::path()
		.and(warp::get())
		.and(warp::query())
		.and(warp::query())
		.and(authorization::authorized(db, Permission::ReportsList.into()))
		.then(move |report: ReportHistorySpecifier, pagination: PaginationOptions<_>, filter: ReportFilter, _, connection: DbConn| async move {
			let page = pagination.page;
			let limit = pagination.limit
				.unwrap_or(CONFIG.default_page_item_limit)
				.clamp(1, CONFIG.max_page_item_limit);

			let page = connection.list_report_history(
				&report,
				page,
				limit,
				filter,
			).await?;
			
			Ok::<_, StatusCode>(warp::reply::json(&page))
		})
}
