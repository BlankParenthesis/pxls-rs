use async_trait::async_trait;
use sea_orm_migration::prelude::*;

use super::{id, col};

#[derive(Iden)]
enum Board {
	Table,
	Id,
	Name,
	#[iden = "created_at"]
	CreatedAt,
	Shape,
	Mask,
	Initial,
}


#[derive(Iden)]
pub enum Color {
	Table,
	Board,
	Index,
	Name,
	Value,
}

#[derive(Iden)]
pub enum Placement {
	Table,
	Id,
	Board,
	Position,
	Color,
	Timestamp,
	#[iden = "user_id"]
	UserId,
}

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait]
impl MigrationTrait for Migration {
	async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		let board_table = Table::create()
			.table(Board::Table)
			.col(id!(Board::Id).integer())
			.col(col!(Board::Name).text())
			.col(col!(Board::CreatedAt).big_integer())
			.col(col!(Board::Shape).text())
			.col(col!(Board::Mask).binary())
			.col(col!(Board::Initial).binary())
			.to_owned();

		let color_table = Table::create()
			.table(Color::Table)
			.col(col!(Color::Board).integer())
			.col(col!(Color::Index).integer())
			.col(col!(Color::Name).text())
			.col(col!(Color::Value).integer())
			.primary_key(Index::create().col(Color::Board).col(Color::Index))
			.foreign_key(ForeignKey::create().from_col(Color::Board).to_tbl(Board::Table).to_col(Board::Id))
			.to_owned();

		let placement_table = Table::create()
			.table(Placement::Table)
			.col(id!(Placement::Id).big_integer())
			.col(col!(Placement::Board).integer())
			.col(col!(Placement::Position).big_integer())
			.col(col!(Placement::Color).small_integer())
			.col(col!(Placement::Timestamp).integer())
			.col(ColumnDef::new(Placement::UserId).null().text())
			.foreign_key(ForeignKey::create().from_col(Placement::Board).to_tbl(Board::Table).to_col(Board::Id))
			.foreign_key(ForeignKey::create()
				.from_col(Placement::Board).from_col(Placement::Color)
				.to_tbl(Color::Table).to_col(Color::Board).to_col(Color::Index))
			.to_owned();

		manager.create_table(board_table).await?;
		manager.create_table(color_table).await?;
		manager.create_table(placement_table).await?;

		Ok(())
	}

	async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		manager.drop_table(Table::drop().table(Placement::Table).to_owned()).await?;
		manager.drop_table(Table::drop().table(Color::Table).to_owned()).await?;
		manager.drop_table(Table::drop().table(Board::Table).to_owned()).await?;

		Ok(())
	}
}