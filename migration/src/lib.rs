pub use sea_orm_migration::prelude::*;


mod m0_create_boards_colors_placements;
mod m1_advanced_shapes;
mod m2_max_stacked_board_property;
mod m3_json_shape;
mod m4_system_colors;

pub struct Migrator;

macro_rules! col {
	($name:expr) => {
		sea_orm_migration::prelude::ColumnDef::new($name).not_null()
	}
}

macro_rules! id {
	($name:expr) => {
		sea_orm_migration::prelude::ColumnDef::new($name).auto_increment().primary_key()
	}
}

use {col, id};

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
	fn migrations() -> Vec<Box<dyn MigrationTrait>> {
		vec![
			Box::new(m0_create_boards_colors_placements::Migration),
			Box::new(m1_advanced_shapes::Migration),
			Box::new(m2_max_stacked_board_property::Migration),
			Box::new(m3_json_shape::Migration),
			Box::new(m4_system_colors::Migration),
		]
	}
}