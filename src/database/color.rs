use std::collections::HashMap;
use std::ops::Not;

use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, Set, SqlErr, StreamTrait, TransactionTrait};
use serde::{Deserialize, Serialize};

use super::entities::*;

use super::{Connection, DbResult, DatabaseError};

// TODO: maybe move shape here too?

pub type Palette = HashMap<u32, Color>;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Color {
	pub name: String,
	pub value: u32,
	#[serde(default)]
	#[serde(skip_serializing_if = "<&bool>::not")]
	pub system_only: bool,
}

impl From<color::Model> for Color {
	fn from(model: color::Model) -> Self {
		Color {
			name: model.name,
			value: model.value as u32,
			system_only: model.system_only,
		}
	}
}

impl<C: TransactionTrait + ConnectionTrait + StreamTrait> Connection<C> {
	pub async fn replace_palette(
		&self,
		palette: Palette,
		board_id: i32,
	) -> DbResult<()> {
		let transaction = self.begin().await?;

		color::Entity::delete_many()
			.filter(color::Column::Board.eq(board_id))
			.exec(&transaction.connection).await?;
	
		for (index, Color { name, value, system_only }) in palette {
			let color = color::ActiveModel {
				board: Set(board_id),
				index: Set(index as i32),
				name: Set(name.clone()),
				value: Set(value as i32),
				system_only: Set(system_only),
			};
	
			color::Entity::insert(color)
				.exec(&transaction.connection).await?;
		}
		
		match transaction.commit().await {
			Err(DatabaseError::DbErr(err)) => {
				if let Some(SqlErr::ForeignKeyConstraintViolation(_)) = err.sql_err() {
					// TODO: This is a user error (the new palette removes
					// colors which are currently used). It should either be
					// passed back up from here, or detected earlier and this
					// is essential asserted as unreachable.
					// Consequently, it needs to return 409 or something.
					Err(DatabaseError::DbErr(err))
				} else {
					Err(DatabaseError::DbErr(err))
				}
			},
			other => other,
		}
	}
}
