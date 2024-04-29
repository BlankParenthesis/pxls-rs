use sea_orm_migration::prelude::*;

use super::{col, id};

#[derive(Iden)]
enum Board {
	Table,
	Id,
}

#[derive(Iden)]
enum BoardNotice {
	Table,
	Id,
	Board,
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
			.table(BoardNotice::Table)
			.col(id!(BoardNotice::Id).integer())
			.col(col!(BoardNotice::Board).integer())
			.col(col!(BoardNotice::Title).string())
			.col(col!(BoardNotice::Content).string())
			.col(col!(BoardNotice::CreatedAt).big_integer())
			.col(ColumnDef::new(BoardNotice::ExpiresAt).big_integer())
			.col(ColumnDef::new(BoardNotice::Author).string())
			.foreign_key(ForeignKey::create().from_col(BoardNotice::Board).to_tbl(Board::Table).to_col(Board::Id))
			.to_owned();
		
		manager.create_table(create_notices).await?;

		Ok(())
	}

	async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		let drop_notices = Table::drop()
			.table(BoardNotice::Table)
			.to_owned();
		
		manager.drop_table(drop_notices).await?;

		Ok(())
	}
}
