use async_trait::async_trait;
use sea_orm_migration::prelude::*;

use super::{id, col};

mod Board {
	use super::*;

	pub struct Table;
	impl Iden for Table {
		fn unquoted(&self,s: &mut dyn std::fmt::Write) {
			write!(s, "board");
		}
	}
	
	#[derive(Iden)]
	pub enum Col {
		Id,
		Name,
		#[iden = "created_at"]
		CreatedAt,
		Shape,
		Mask,
		Initial,
	}
}

mod Color {
	use super::*;

	pub struct Table;
	impl Iden for Table {
		fn unquoted(&self,s: &mut dyn std::fmt::Write) {
			write!(s, "color");
		}
	}
	
	#[derive(Iden)]
	pub enum Col {
		Board,
		Index,
		Name,
		Value,
	}
}

mod Placement {
	use super::*;

	pub struct Table;
	impl Iden for Table {
		fn unquoted(&self,s: &mut dyn std::fmt::Write) {
			write!(s, "placement");
		}
	}
	
	#[derive(Iden)]
	pub enum Col {
		Id,
		Board,
		Position,
		Color,
		Timestamp,
		#[iden = "user_id"]
		UserId,
	}
}

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait]
impl MigrationTrait for Migration {
	async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		let board_table = Table::create()
			.table(Board::Table)
			.col(id!(Board::Col::Id).integer())
			.col(col!(Board::Col::Name).text())
			.col(col!(Board::Col::CreatedAt).big_integer())
			.col(col!(Board::Col::Shape).text())
			.col(col!(Board::Col::Mask).binary())
			.col(col!(Board::Col::Initial).binary())
			.to_owned();

		let color_table = Table::create()
			.table(Color::Table)
			.col(col!(Color::Col::Board).integer())
			.col(col!(Color::Col::Index).integer())
			.col(col!(Color::Col::Name).text())
			.col(col!(Color::Col::Value).integer())
			.primary_key(Index::create().col(Color::Col::Board).col(Color::Col::Index))
			.foreign_key(ForeignKey::create().from_col(Color::Col::Board).to_tbl(Board::Table).to_col(Board::Col::Id))
			.to_owned();

		let placement_table = Table::create()
			.table(Placement::Table)
			.col(id!(Placement::Col::Id).big_integer())
			.col(col!(Placement::Col::Board).integer())
			.col(col!(Placement::Col::Position).big_integer())
			.col(col!(Placement::Col::Color).small_integer())
			.col(col!(Placement::Col::Timestamp).integer())
			.col(col!(Placement::Col::UserId).text())
			.foreign_key(ForeignKey::create().from_col(Placement::Col::Board).to_tbl(Board::Table).to_col(Board::Col::Id))
			.foreign_key(ForeignKey::create()
				.from_col(Placement::Col::Board).from_col(Placement::Col::Color)
				.to_tbl(Color::Table).to_col(Color::Col::Board).to_col(Color::Col::Index))
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