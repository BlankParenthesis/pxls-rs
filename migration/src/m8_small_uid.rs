use sea_orm_migration::prelude::*;

use super::{col, id};

#[derive(Iden)]
enum Placement {
	Table,
	#[iden = "user_id"]
	UserId,
	#[iden = "new_user_id"]
	NewUserId,
}

#[derive(Iden)]
enum UserId {
	Table,
	Id,
	Uid
}

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
	async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		let create_uid_mapping_table = Table::create()
			.table(UserId::Table)
			.col(id!(UserId::Id).integer())
			.col(col!(UserId::Uid).string().unique_key())
			.to_owned();

		let copy_uid_data = Query::insert()
			.into_table(UserId::Table)
			.columns([UserId::Uid])
			.select_from(
				Query::select()
					.distinct()
					.column(Placement::UserId)
					.from(Placement::Table)
					.to_owned()
			).unwrap()
			.to_owned();
		
		let alter_placement_add_new_ids = Table::alter()
			.table(Placement::Table)
			.add_column(ColumnDef::new(Placement::NewUserId).null().integer())
			.to_owned();
		
		let insert_placement_populate_new_ids = Query::update()
			.table(Placement::Table)
			.value(
				Placement::NewUserId, 
				SimpleExpr::SubQuery(None, Box::new(
					Query::select()
						.column(UserId::Id)
						.from(UserId::Table)
						.and_where(
							SimpleExpr::Column(UserId::Uid.into_column_ref())
								.eq(Placement::UserId.into_column_ref())
						)
						.to_owned()
						.into_sub_query_statement()
				))
			)
			.to_owned();
		
		let alter_placement_drop_old_ids = Table::alter()
			.table(Placement::Table)
			.drop_column(Placement::UserId)
			.to_owned();

		let alter_placement_rename = Table::alter()
			.table(Placement::Table)
			.rename_column(Placement::NewUserId, Placement::UserId)
			.to_owned();
		
		let alter_placement_not_null = Table::alter()
			.table(Placement::Table)			.table(Placement::Table)
			.modify_column(col!(Placement::UserId).integer())
			.to_owned();

		let alter_placement_foreign_key = Table::alter()
			.table(Placement::Table)
			.add_foreign_key(
				TableForeignKey::new()
					.from_tbl(Placement::Table)
					.from_col(Placement::UserId)
					.to_tbl(UserId::Table)
					.to_col(UserId::Id)
			)
			.to_owned();
		
		manager.create_table(create_uid_mapping_table).await?;
		manager.exec_stmt(copy_uid_data).await?;
		manager.alter_table(alter_placement_add_new_ids).await?;
		manager.exec_stmt(insert_placement_populate_new_ids).await?;
		manager.alter_table(alter_placement_drop_old_ids).await?;
		manager.alter_table(alter_placement_rename).await?;
		manager.alter_table(alter_placement_not_null).await?;
		manager.alter_table(alter_placement_foreign_key).await?;

		Ok(())
	}

	async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {

		let alter_placement_rename_new_uid = Table::alter()
			.table(Placement::Table)
			.rename_column(Placement::UserId, Placement::NewUserId)
			.to_owned();

		let alter_placement_add_uid = Table::alter()
			.table(Placement::Table)
			.add_column(ColumnDef::new(Placement::UserId).null().text())
			.to_owned();

		
		let insert_placement_populate_ids = Query::update()
			.table(Placement::Table)
			.value(
				Placement::UserId, 
				SimpleExpr::SubQuery(None, Box::new(
					Query::select()
						.column(UserId::Uid)
						.from(UserId::Table)
						.and_where(
							SimpleExpr::Column(UserId::Id.into_column_ref())
								.eq(Placement::NewUserId.into_column_ref())
						)
						.to_owned()
						.into_sub_query_statement()
				))
			)
			.to_owned();

		let drop_placement_userid = Table::alter()
			.table(Placement::Table)
			.drop_column(Placement::NewUserId)
			.to_owned();

		let drop_userid = Table::drop()
			.table(UserId::Table)
			.to_owned();

		manager.alter_table(alter_placement_rename_new_uid).await?;
		manager.alter_table(alter_placement_add_uid).await?;
		manager.exec_stmt(insert_placement_populate_ids).await?;
		manager.alter_table(drop_placement_userid).await?;
		manager.drop_table(drop_userid).await?;

		Ok(())
	}
}
