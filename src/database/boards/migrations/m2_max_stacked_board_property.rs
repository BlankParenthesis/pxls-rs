use sea_orm_migration::prelude::*;

use super::col;

#[derive(Iden)]
enum Board {
	Table,
	#[iden = "max_stacked"]
	MaxStacked,
}

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
	async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		let create_max_stacked = Table::alter()
			.table(Board::Table)
			.add_column(col!(Board::MaxStacked).integer().default(6))
			.to_owned();

		let remove_default = Table::alter()
			.table(Board::Table)
			.modify_column(col!(Board::MaxStacked).integer())
			.to_owned();
		
		manager.alter_table(create_max_stacked).await?;
		manager.alter_table(remove_default).await?;

		Ok(())
	}

	async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		let drop_stacked = Table::alter()
			.table(Board::Table)
			.drop_column(Board::MaxStacked)
			.to_owned();
		
		manager.alter_table(drop_stacked).await?;

		Ok(())
	}
}
