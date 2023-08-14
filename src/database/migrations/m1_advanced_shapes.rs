use async_trait::async_trait;
use sea_orm_migration::prelude::*;
use super::col;

#[derive(Iden)]
enum Board {
	Table,
	Id,
	#[iden = "created_at"]
	Shape,
	#[iden = "shape_new"]
	ShapeNew,
	#[iden = "shape_old"]
	ShapeOld,
	Mask,
	Initial,
}


#[derive(Iden)]
enum BoardSector {
	Table,
	Board,
	Sector,
	Mask,
	Initial,
}

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait]
impl MigrationTrait for Migration {
	async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		let board_sector_table = Table::create()
			.table(BoardSector::Table)
			.col(col!(BoardSector::Board).integer())
			.col(ColumnDef::new(BoardSector::Sector).integer().default(0))
			.col(col!(BoardSector::Mask).binary())
			.col(col!(BoardSector::Initial).binary())
			.primary_key(Index::create().col(BoardSector::Board).col(BoardSector::Sector))
			.foreign_key(ForeignKey::create().from_col(BoardSector::Board).to_tbl(Board::Table).to_col(Board::Id))
			.to_owned();

		let board_to_sector = Query::insert()
			.into_table(BoardSector::Table)
			.columns([BoardSector::Board, BoardSector::Mask, BoardSector::Initial])
			.select_from(
				Query::select()
					.columns([Board::Id, Board::Mask, Board::Initial])
					.from(Board::Table)
					.to_owned()
			)
			.unwrap()
			.to_owned();

		let make_sector_non_null = Table::alter()
			.table(BoardSector::Table)
			.modify_column(col!(BoardSector::Sector).integer())
			.to_owned();

		let drop_sector_data = Table::alter()
			.table(Board::Table)
			.drop_column(Board::Mask)
			.drop_column(Board::Initial)
			.to_owned();

		let add_new_shape = Table::alter()
			.table(Board::Table)
			.add_column(ColumnDef::new(Board::ShapeNew).array(ColumnType::Array(std::sync::Arc::new(ColumnType::Integer))))
			.to_owned();

		let update_board_map_shape = Query::update()
			.table(Board::Table)
			.value(Board::ShapeNew, Expr::cust("CAST(REPLACE(REPLACE(\"shape\", '[', '{'), ']', '}') AS INTEGER[][])"))
			.to_owned();

		let make_newshape_non_null = Table::alter()
			.table(Board::Table)
			.modify_column(col!(Board::ShapeNew).array(ColumnType::Array(std::sync::Arc::new(ColumnType::Integer))))
			.to_owned();

		let drop_shape = Table::alter()
			.table(Board::Table)
			.drop_column(Board::Shape)
			.to_owned();

		let rename_newshape = Table::alter()
			.table(Board::Table)
			.rename_column(Board::ShapeNew, Board::Shape)
			.to_owned();
		
		manager.create_table(board_sector_table).await?;
		manager.exec_stmt(board_to_sector).await?;
		manager.alter_table(make_sector_non_null).await?;
		manager.alter_table(drop_sector_data).await?;
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
			.add_column(ColumnDef::new(Board::ShapeOld).text())
			.to_owned();

		// NOTE: takes the last shape element since that's what should
		// match the sector data sizes. 
		// If the number of dimensions is not two, the server is going
		// to have a bad time regardless.

		let revert_shape = Query::update()
			.table(Board::Table).value(Board::ShapeOld, Expr::cust("'[' || CAST(TO_JSON(\"shape\")->-1 as TEXT) || ']'"))
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

		let revert_board = Table::alter()
			.table(Board::Table)
			.add_column(ColumnDef::new(Board::Mask).binary())
			.add_column(ColumnDef::new(Board::Initial).binary())
			.to_owned();

		// This is missing the required "FROM" clause
		//let unmove_data = Query::update()
		//	.table(Board::Table)
		//	.values([
		//		(Board::Mask, Expr::col((BoardSector::Table, BoardSector::Mask)).into()),
		//		(Board::Initial, Expr::col((BoardSector::Table, BoardSector::Initial)).into()),
		//	])
		//	.and_where(
		//		Expr::col((Board::Table, Board::Id)).eq(Expr::col((BoardSector::Table, BoardSector::Board)))
		//		.and(Expr::col((BoardSector::Table, BoardSector::Sector)).eq(0))
		//	)
		//	.to_owned();

		let unmove_data = r#"
			UPDATE "board"
			SET "mask" = "board_sector"."mask",
				"initial" = "board_sector"."initial"
			FROM "board_sector"
			WHERE "board"."id" = "board_sector"."board"
			AND "board_sector"."sector" = 0;
		"#;

		let make_data_non_null = Table::alter()
			.table(Board::Table)
			.modify_column(col!(Board::Mask).binary())
			.modify_column(col!(Board::Initial).binary())
			.to_owned();

		let drop_sector = Table::drop().table(BoardSector::Table).to_owned();
		
		manager.alter_table(add_old_shape).await?;
		manager.exec_stmt(revert_shape).await?;
		manager.alter_table(make_oldshape_non_null).await?;
		manager.alter_table(drop_shape).await?;
		manager.alter_table(rename_oldshape).await?;
		manager.alter_table(revert_board).await?;
		manager.get_connection().execute_unprepared(unmove_data).await?;
		manager.alter_table(make_data_non_null).await?;
		manager.drop_table(drop_sector).await?;

		Ok(())
	}
}