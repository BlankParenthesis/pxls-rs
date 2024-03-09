use sea_orm_migration::prelude::*;

use super::col;

#[derive(Iden)]
enum Color {
	Table,
	#[iden = "system_only"]
	SystemOnly,
}

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
	async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		let create_system_only = Table::alter()
			.table(Color::Table)
			.add_column(col!(Color::SystemOnly).boolean().default(false))
			.to_owned();
		
		manager.alter_table(create_system_only).await?;

		Ok(())
	}

	async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		let drop_system_only = Table::alter()
			.table(Color::Table)
			.drop_column(Color::SystemOnly)
			.to_owned();
		
		manager.alter_table(drop_system_only).await?;

		Ok(())
	}
}
