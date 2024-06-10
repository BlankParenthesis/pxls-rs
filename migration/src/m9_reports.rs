use sea_orm_migration::prelude::*;

use super::col;

#[derive(Iden)]
enum Report {
	Table,
	Id,
	Revision,
	Closed,
	Reason,
	Reporter,
	Timestamp,
}

#[derive(Iden)]
enum ReportArtifact {
	Table,
	Report,
	Revision,
	Timestamp,
	Uri,
}

#[derive(Iden)]
enum UserId {
	Table,
	Id,
}

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
	async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		let create_reports = Table::create()
			.table(Report::Table)
			.col(col!(Report::Id).integer().auto_increment())
			.col(col!(Report::Revision).integer())
			.col(col!(Report::Closed).boolean())
			.col(col!(Report::Reason).string())
			.col(col!(Report::Timestamp).big_integer())
			.col(ColumnDef::new(Report::Reporter).integer())
			.primary_key(Index::create().col(Report::Id).col(Report::Revision))
			.foreign_key(ForeignKey::create().from_col(Report::Reporter).to_tbl(UserId::Table).to_col(UserId::Id))
			.to_owned();

		let create_artifacts = Table::create()
			.table(ReportArtifact::Table)
			.col(col!(ReportArtifact::Report).integer())
			.col(col!(ReportArtifact::Revision).integer())
			.col(col!(ReportArtifact::Timestamp).big_integer())
			.col(col!(ReportArtifact::Uri).string())
			.primary_key(Index::create().col(ReportArtifact::Report).col(ReportArtifact::Revision).col(ReportArtifact::Uri).col(ReportArtifact::Timestamp))
			.foreign_key(ForeignKey::create().from(ReportArtifact::Table, (ReportArtifact::Report, ReportArtifact::Revision))
				.to(Report::Table, (Report::Id, Report::Revision)))
			.to_owned();

		manager.create_table(create_reports).await?;
		manager.create_table(create_artifacts).await?;

		Ok(())
	}

	async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		let drop_artifacts = Table::drop()
			.table(ReportArtifact::Table)
			.to_owned();

		let drop_reports = Table::drop()
			.table(Report::Table)
			.to_owned();
		
		manager.drop_table(drop_artifacts).await?;
		manager.drop_table(drop_reports).await?;

		Ok(())
	}
}
