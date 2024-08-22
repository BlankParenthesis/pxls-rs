use sea_orm_migration::prelude::*;

#[derive(Iden)]
enum Placement {
	Table,
	Board,
	Color,
}

#[derive(Iden)]
enum Color {
	Table,
	Board,
	Index,
}

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
	async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		let remove_old = Table::alter()
			.table(Placement::Table)
			.drop_foreign_key(Alias::new("placement_board_color_fkey"))
			.to_owned();

		manager.alter_table(remove_old).await?;

		manager.get_connection().execute_unprepared(
			"ALTER TABLE placement ADD CONSTRAINT \"placement_board_color_fkey\" FOREIGN KEY (board, color) REFERENCES color(board, index) DEFERRABLE INITIALLY DEFERRED"
		).await?;

		Ok(())
	}

	async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		let remove_deferred = Table::alter()
			.table(Placement::Table)
			.drop_foreign_key(Alias::new("placement_board_color_fkey"))
			.to_owned();
		
		let add_old = Table::alter()
			.table(Placement::Table)
			.add_foreign_key(TableForeignKey::new()
				.from_tbl(Placement::Table).from_col(Placement::Board).from_col(Placement::Color)
				.to_tbl(Color::Table).to_col(Color::Board).to_col(Color::Index))
			.to_owned();
		
		manager.alter_table(remove_deferred).await?;
		manager.alter_table(add_old).await?;

		Ok(())
	}
}
