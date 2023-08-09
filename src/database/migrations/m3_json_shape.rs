use sea_orm_migration::prelude::*;

use super::col;

#[derive(Iden)]
enum Board {
	Table,
	Shape,
	#[iden = "shape_new"]
	ShapeNew,
	#[iden = "shape_old"]
	ShapeOld,
}

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
	async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		let add_new_shape = Table::alter()
			.table(Board::Table)
			.add_column(ColumnDef::new(Board::ShapeNew).json_binary())
			.to_owned();

		let update_board_map_shape = Query::update()
			.table(Board::Table)
			.value(Board::ShapeNew, Expr::cust(
				format!("to_jsonb(\"{}\")", Board::Shape.to_string()),
			))
			.to_owned();

		let make_newshape_non_null = Table::alter()
			.table(Board::Table)
			.modify_column(col!(Board::ShapeNew).json_binary())
			.to_owned();

		let drop_shape = Table::alter()
			.table(Board::Table)
			.drop_column(Board::Shape)
			.to_owned();

		let rename_newshape = Table::alter()
			.table(Board::Table)
			.rename_column(Board::ShapeNew, Board::Shape)
			.to_owned();
			

		manager.alter_table(add_new_shape).await?;
		manager.exec_stmt(update_board_map_shape).await?;
		manager.alter_table(make_newshape_non_null).await?;
		manager.alter_table(drop_shape).await?;
		manager.alter_table(rename_newshape).await?;

		Ok(())
	}

	async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		let add_old_shape = Table::alter()
			.table(Board::Table)
			.add_column(ColumnDef::new(Board::ShapeOld).array(
				ColumnType::Array(std::sync::Arc::new(ColumnType::Integer))
			))
			.to_owned();

		let revert_shape = Query::update()
			.value(Board::ShapeOld, Expr::cust(format!(
				"CAST(REPLACE(REPLACE(CAST(\"{}\" AS TEXT), '[', '{{'), ']', '}}') AS INTEGER[][])",
				Board::Shape.to_string()
			)))
			.to_owned();

		let make_oldshape_non_null = Table::alter()
			.table(Board::Table)
			.modify_column(col!(Board::ShapeOld).text())
			.to_owned();

		let drop_shape = Table::alter()
			.table(Board::Table)
			.drop_column(Board::Shape)
			.to_owned();

		let rename_oldshape = Table::alter()
			.table(Board::Table)
			.rename_column(Board::ShapeOld, Board::Shape)
			.to_owned();

		manager.alter_table(add_old_shape).await?;
		manager.exec_stmt(revert_shape).await?;
		manager.alter_table(make_oldshape_non_null).await?;
		manager.alter_table(drop_shape).await?;
		manager.alter_table(rename_oldshape).await?;

		Ok(())
	}
}
