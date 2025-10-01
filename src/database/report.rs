use std::time::{SystemTime, UNIX_EPOCH};
use std::fmt;

use sea_orm::TryInsertResult;
use sea_query::SimpleExpr;
use warp::http::Uri;
use sea_orm::{ActiveValue::NotSet, ColumnTrait, ConnectionTrait, DbErr, EntityTrait, ModelTrait, QueryFilter, QueryOrder, QuerySelect, QueryTrait, Set, StreamTrait, TransactionTrait};
use serde::{Serialize, Deserialize};

use crate::filter::response::paginated_list::{Page, PageToken};
use crate::filter::response::reference::{Referencable, Reference};

use super::entities::*;

use super::{Connection, DbResult, DbInsertResult, DatabaseError, InsertError};
use super::user::{User, UserSpecifier};
use super::specifier::{SpecifierParser, Id, SpecfierParseError, Specifier, PathPart, specifier_path};

#[derive(Debug, Clone, Copy)]
pub struct ReportSpecifier(i32);

impl Specifier for ReportSpecifier {
	fn filter(&self) -> SimpleExpr {
		report::Column::Id.eq(self.0)
	}
	
	fn from_ids(ids: &[&str]) -> Result<Self, SpecfierParseError> {
		let report = ids[0].parse()?;
		Ok(Self(report))
	}
	
	fn to_ids(&self) -> Box<[Id]> {
		Box::new([Id::I32(self.0)])
	}
	
	fn parts() -> &'static [PathPart] {
		specifier_path!("reports", report)
	}
}

impl<'de> Deserialize<'de> for ReportSpecifier {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where D: serde::Deserializer<'de> {
		deserializer.deserialize_str(SpecifierParser::new("A report uri"))
	}
}

impl Serialize for ReportSpecifier {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where S: serde::Serializer {
		serializer.serialize_str(self.to_uri().path())
	}
}

#[derive(Debug, Clone, Copy)]
pub struct ReportHistorySpecifier(i32);

impl Specifier for ReportHistorySpecifier {
	fn filter(&self) -> SimpleExpr {
		report::Column::Id.eq(self.0)
	}
	
	fn from_ids(ids: &[&str]) -> Result<Self, SpecfierParseError> {
		let report = ids[0].parse()?;
		Ok(Self(report))
	}
	
	fn to_ids(&self) -> Box<[Id]> {
		Box::new([Id::I32(self.0)])
	}
	
	fn parts() -> &'static [PathPart] {
		specifier_path!("reports", report, "history")
	}
}

impl<'de> Deserialize<'de> for ReportHistorySpecifier {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where D: serde::Deserializer<'de> {
		deserializer.deserialize_str(SpecifierParser::new("A report history uri"))
	}
}

impl Serialize for ReportHistorySpecifier {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where S: serde::Serializer {
		serializer.serialize_str(self.to_uri().path())
	}
}

#[serde_with::skip_serializing_none]
#[derive(Serialize, Debug, Clone)]
pub struct Report {
	#[serde(skip_serializing)]
	id: ReportSpecifier,
	pub status: ReportStatus,
	pub reason: String,
	pub artifacts: Vec<Artifact>,
	pub reporter: Option<Reference<User>>,
	pub timestamp: u64,
}

impl Referencable for Report {
	fn uri(&self) -> Uri {
		self.id.to_uri()
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

// TODO: this needs more work; specifiers? validation? more objects?
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
			reference: Reference::new_empty(uri),
			r#type: artifact_type,
			timestamp,
		})
	}
}


impl<C: TransactionTrait + ConnectionTrait + StreamTrait> Connection<C> {
	
	pub async fn list_reports(
		&self,
		token: ReportPageToken,
		limit: usize,
		filter: ReportFilter,
		owner: Option<Option<&UserSpecifier>>,
	) -> DbResult<Page<Reference<Report>>> {
		let transaction = self.connection.begin().await?;

		let list = report::Entity::find()
			.find_also_related(user::Entity)
			.distinct_on([report::Column::Id])
			.filter(report::Column::Id.gt(token.0 as i64))
			.apply_if(filter.status.as_ref(), |q, status| q.filter(report::Column::Closed.eq(matches!(status, ReportStatus::Closed))))
			.apply_if(filter.reason.as_ref(), |q, reason| q.filter(report::Column::Reason.eq(reason)))
			.apply_if(owner, |q, owner| q.filter(report::Column::Reporter.eq(owner.map(|o| o.0))))
			.order_by(report::Column::Id, sea_orm::Order::Asc)
			.order_by(report::Column::Revision, sea_orm::Order::Desc)
			.limit(limit as u64 + 1)
			.all(&transaction).await?;

		let next = list.windows(2).nth(limit.saturating_sub(1))
			.map(|pair| &pair[0])
			.map(|(report, _)| ReportPageToken(report.id as _))
			.map(|token| {
				format!( // TODO: filter
					"/reports?page={}&limit={}",
					token, limit,
				).parse().unwrap()
			});

		let mut reports = vec![];

		for (report, reporter) in list.into_iter().take(limit) {
			// TODO: do this in one query
			let artifacts = report.find_related(report_artifact::Entity)
				.all(&transaction).await?
				.into_iter()
				.map(|a| Artifact::parse(&a.uri, a.timestamp as _))
				.collect::<Result<_, _>>()
				.map_err(|_| sea_orm::DbErr::Custom("integrity error".to_string()))?;

			let report = Report {
				id: ReportSpecifier(report.id),
				status: if report.closed { ReportStatus::Closed } else { ReportStatus::Opened },
				reason: report.reason,
				reporter: reporter.map(User::from).map(Reference::from),
				artifacts,
				timestamp: report.timestamp as _,
			};
			reports.push(Reference::from(report))
		}

		transaction.commit().await?;
		
		Ok(Page { items: reports, next, previous: None })
	}

	pub async fn get_report(
		&self,
		report: &ReportSpecifier,
	) -> DbResult<Option<Report>> {
		let transaction = self.connection.begin().await?;

		let report = report::Entity::find()
			.find_also_related(user::Entity)
			.filter(report::Column::Id.eq(report.0))
			.order_by(report::Column::Revision, sea_orm::Order::Desc)
			.limit(1)
			.one(&transaction).await
			.map_err(DatabaseError::from)?;
		
		match report {
			Some((report, reporter)) => {
				let artifacts = report.find_related(report_artifact::Entity)
					.all(&transaction).await?
					.into_iter()
					.map(|a| Artifact::parse(&a.uri, a.timestamp as _))
					.collect::<Result<_, _>>()
					.map_err(|_| sea_orm::DbErr::Custom("integrity error".to_string()))?;

				transaction.commit().await?;

				let report = Report {
					id: ReportSpecifier(report.id),
					status: if report.closed { ReportStatus::Closed } else { ReportStatus::Opened },
					reason: report.reason,
					reporter: reporter.map(User::from).map(Reference::from),
					artifacts,
					timestamp: report.timestamp as _,
				};
				Ok(Some(report))
			},
			None => Ok(None),
		}
	}

	pub async fn create_report(
		&self,
		reason: String,
		reporter: Option<&UserSpecifier>,
		artifacts: Vec<Artifact>,
	) -> DbInsertResult<Report> {
		let now = SystemTime::now()
			.duration_since(UNIX_EPOCH)
			.unwrap()
			.as_secs();
		
		let model = report::ActiveModel {
			id: NotSet,
			revision: Set(1),
			closed: Set(false),
			reason: Set(reason),
			reporter: Set(reporter.map(|r| r.0)),
			timestamp: Set(now as _),
		};
		
		let transaction = self.begin().await?;
		
		let reporter = if let Some(user) = reporter {
			let user = transaction.get_user(user).await?
				.ok_or(InsertError::MissingDependency)?;
			Some(user)
		} else {
			None
		};

		let insert = report::Entity::insert(model)
			.do_nothing()
			.exec_with_returning(&transaction.connection).await?;
		
		let report = match insert {
			TryInsertResult::Inserted(insert) => insert,
			TryInsertResult::Empty => return Err(InsertError::MissingDependency),
			TryInsertResult::Conflicted => unreachable!("conflict on insert report without set key"),
		};
		
		let artifact_models = artifacts.iter().map(|a| {
			report_artifact::ActiveModel {
				report: Set(report.id),
				revision: Set(report.revision),
				timestamp: Set(a.timestamp as _),
				uri: Set(a.reference.uri.to_string()),
			}
		});

		report_artifact::Entity::insert_many(artifact_models)
			.exec(&transaction.connection).await?;
		
		transaction.commit().await?;
		
		let report = Report {
			id: ReportSpecifier(report.id),
			status: if report.closed { ReportStatus::Closed } else { ReportStatus::Opened },
			reason: report.reason,
			reporter: reporter.map(|r| r.clone().into()),
			artifacts,
			timestamp: report.timestamp as _,
		};
		Ok(report)
	}

	pub async fn edit_report(
		&self,
		report: &ReportSpecifier,
		status: Option<ReportStatus>,
		reason: Option<String>,
		artifacts: Option<Vec<Artifact>>,
	) -> DbResult<Report> {
		let now = SystemTime::now()
			.duration_since(UNIX_EPOCH)
			.unwrap()
			.as_secs();

		let transaction = self.begin().await?;

		let old_report = report::Entity::find()
			.filter(report.filter())
			.order_by(report::Column::Revision, sea_orm::Order::Desc)
			.one(&transaction.connection).await
			.map_err(DatabaseError::from)?
			.ok_or(DatabaseError::DbErr(DbErr::RecordNotFound("".to_string())))?;

		let artifacts = if let Some(a) = artifacts {
			a
		} else {
			report_artifact::Entity::find()
				.filter(report_artifact::Column::Report.eq(old_report.id))
				.filter(report_artifact::Column::Revision.eq(old_report.revision))
				.all(&transaction.connection).await?
				.into_iter()
				.map(|a| Artifact::parse(&a.uri, a.timestamp as _))
				.collect::<Result<_, _>>()
				.map_err(|_| sea_orm::DbErr::Custom("integrity error".to_string()))?
		};

		let closed = status.map(|s| matches!(s, ReportStatus::Closed))
			.unwrap_or(old_report.closed);
		
		let model = report::ActiveModel {
			id: Set(report.0),
			revision: Set(old_report.revision + 1),
			closed: Set(closed),
			reason: Set(reason.unwrap_or(old_report.reason)),
			reporter: Set(old_report.reporter),
			timestamp: Set(now as _),
		};
		
		let report = report::Entity::insert(model)
			.exec_with_returning(&transaction.connection).await?;
		
		let artifact_models = artifacts.iter().map(|a| {
			report_artifact::ActiveModel {
				report: Set(report.id),
				revision: Set(report.revision),
				timestamp: Set(a.timestamp as _),
				uri: Set(a.reference.uri.to_string()),
			}
		});

		report_artifact::Entity::insert_many(artifact_models)
			.do_nothing()
			.exec(&transaction.connection).await?;

		let reporter = if let Some(reporter) = report.reporter {
			let user = transaction.get_user(&UserSpecifier(reporter)).await?
				.expect("failed to lookup reporting user");
			Some(Reference::from(user))
		} else {
			None
		};
		

		transaction.commit().await?;

		let report = Report {
			id: ReportSpecifier(report.id),
			status: if report.closed { ReportStatus::Closed } else { ReportStatus::Opened },
			reason: report.reason,
			reporter,
			artifacts,
			timestamp: report.timestamp as _,
		};
		Ok(report)
	}

	// returns Some(reporter) if the report was deleted or None if it didn't exist
	pub async fn delete_report(
		&self,
		report: ReportSpecifier,
	) -> DbResult<Option<Option<UserSpecifier>>> {
		let transaction = self.connection.begin().await?;

		let reporter_id = report::Entity::find()
			.find_also_related(user::Entity)
			.filter(report.filter())
			.order_by(report::Column::Revision, sea_orm::Order::Desc)
			.limit(1)
			.one(&transaction).await?
			.map(|(_, user)| user.map(|u| UserSpecifier(u.id)));

		let deleted = report::Entity::delete_many()
			.filter(report.filter())
			.exec(&transaction).await
			.map(|result| result.rows_affected > 0)?;

		transaction.commit().await?;

		if deleted {
			Ok(Some(reporter_id.unwrap()))
		} else {
			Ok(None)
		}
	}
	
	
	
	pub async fn list_report_history(
		&self,
		list: &ReportHistorySpecifier,
		token: ReportPageToken,
		limit: usize,
		filter: ReportFilter,
	) -> DbResult<Page<Report>> {
		let transaction = self.connection.begin().await?;

		let reports = report::Entity::find()
			.find_also_related(user::Entity)
			.filter(list.filter())
			.filter(report::Column::Revision.gt(token.0 as i64))
			.apply_if(filter.status.as_ref(), |q, status| q.filter(report::Column::Closed.eq(matches!(status, ReportStatus::Closed))))
			.apply_if(filter.reason.as_ref(), |q, reason| q.filter(report::Column::Reason.eq(reason)))
			.order_by(report::Column::Revision, sea_orm::Order::Asc)
			.limit(limit as u64 + 1)
			.all(&transaction).await?;

		let next = reports.windows(2).nth(limit.saturating_sub(1))
			.map(|pair| &pair[0])
			.map(|(report, _)| ReportPageToken(report.revision as _))
			.map(|token| {
				let uri = list.to_uri();
				let path = uri.path();
				// TODO: filter
				format!("{path}?page={token}&limit={limit}").parse().unwrap()
			});

		let mut items = vec![];

		for (report, reporter) in reports.into_iter().take(limit) {
			let artifacts = report.find_related(report_artifact::Entity)
				.all(&transaction).await?
				.into_iter()
				.map(|a| Artifact::parse(&a.uri, a.timestamp as _))
				.collect::<Result<_, _>>()
				.map_err(|_| sea_orm::DbErr::Custom("integrity error".to_string()))?;

			let report = Report {
				id: ReportSpecifier(report.id),
				status: if report.closed { ReportStatus::Closed } else { ReportStatus::Opened },
				reason: report.reason,
				reporter: reporter.map(User::from).map(Reference::from),
				artifacts,
				timestamp: report.timestamp as _,
			};
			items.push(report)
		}

		transaction.commit().await?;
		
		Ok(Page { items, next, previous: None })
	}
}
