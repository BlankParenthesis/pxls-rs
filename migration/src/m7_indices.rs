use sea_orm_migration::prelude::*;

#[derive(Iden)]
enum Placement {
	Table,
	#[iden = "user_id"]
	UserId,
	Timestamp,
	Position,
}

const PLACEMENT_BY_USER: &str = "placement_by_user";
const PLACEMENT_BY_TIMESTAMP: &str = "placement_by_timestamp";
const PLACEMENT_BY_POSITION: &str = "placement_by_position";

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
	async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		let create_placement_by_user = Index::create()
			.name(PLACEMENT_BY_USER)
			.table(Placement::Table)
			.col(Placement::UserId)
			.to_owned();

		manager.create_index(create_placement_by_user).await?;

		let create_placement_by_timestamp = Index::create()
			.name(PLACEMENT_BY_TIMESTAMP)
			.table(Placement::Table)
			.col(Placement::Timestamp)
			.to_owned();

		manager.create_index(create_placement_by_timestamp).await?;

		let create_placement_by_position = Index::create()
			.name(PLACEMENT_BY_POSITION)
			.table(Placement::Table)
			.col(Placement::Position)
			.to_owned();

		manager.create_index(create_placement_by_position).await?;

		Ok(())
	}

	async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		let drop_placement_by_user = Index::drop()
			.name(PLACEMENT_BY_USER)
			.to_owned();
		
		manager.drop_index(drop_placement_by_user).await?;

		let drop_placement_by_timestamp = Index::drop()
			.name(PLACEMENT_BY_TIMESTAMP)
			.to_owned();
		
		manager.drop_index(drop_placement_by_timestamp).await?;

		let drop_placement_by_position = Index::drop()
			.name(PLACEMENT_BY_POSITION)
			.to_owned();
		
		manager.drop_index(drop_placement_by_position).await?;

		Ok(())
	}
}
