use sea_orm_migration::prelude::*;

use super::{col, id};

#[derive(Iden)]
enum Notice {
	Table,
	Id,
	Title,
	Content,
	#[iden = "created_at"]
	CreatedAt,
	#[iden = "expires_at"]
	ExpiresAt,
	Author,
}

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
	async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		let create_notices = Table::create()
			.table(Notice::Table)
			.col(id!(Notice::Id).integer())
			.col(col!(Notice::Title).string())
			.col(col!(Notice::Content).string())
			.col(col!(Notice::CreatedAt).big_integer())
			.col(ColumnDef::new(Notice::ExpiresAt).big_integer())
			.col(ColumnDef::new(Notice::Author).string())
			.to_owned();
		
		manager.create_table(create_notices).await?;

		Ok(())
	}

	async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		let drop_notices = Table::drop()
			.table(Notice::Table)
			.to_owned();
		
		manager.drop_table(drop_notices).await?;

		Ok(())
	}
}
