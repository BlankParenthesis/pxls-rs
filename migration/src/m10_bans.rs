use sea_orm_migration::prelude::*;

use super::{col, id};

#[derive(Iden)]
enum Ban {
	Table,
	Id,
	#[iden = "user_id"]
	UserId,
	#[iden = "created_at"]
	CreatedAt,
	#[iden = "expires_at"]
	ExpiresAt,
	Issuer,
	Reason,
}

#[derive(Iden)]
enum UserId {
	Table,
	Id,
}

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
	async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		let create_bans = Table::create()
			.table(Ban::Table)
			.col(id!(Ban::Id).integer())
			.col(col!(Ban::UserId).integer())
			.col(col!(Ban::CreatedAt).big_integer())
			.col(ColumnDef::new(Ban::ExpiresAt).null().big_integer())
			.col(ColumnDef::new(Ban::Issuer).null().integer())
			.col(ColumnDef::new(Ban::Reason).null().text())
			.foreign_key(ForeignKey::create().from_col(Ban::UserId).to_tbl(UserId::Table).to_col(UserId::Id))
			.foreign_key(ForeignKey::create().from_col(Ban::Issuer).to_tbl(UserId::Table).to_col(UserId::Id))
			.to_owned();

		manager.create_table(create_bans).await?;

		Ok(())
	}

	async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		let drop_bans = Table::drop()
			.table(Ban::Table)
			.to_owned();
		
		manager.drop_table(drop_bans).await?;

		Ok(())
	}
}
