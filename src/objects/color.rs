use std::collections::HashMap;

use sea_orm::{TransactionTrait, EntityTrait, Set, ColumnTrait, QueryFilter};
use serde::{Deserialize, Serialize};

use crate::database::{entities::*, DbResult};

pub type Palette = HashMap<u32, Color>;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Color {
	pub name: String,
	pub value: u32,
}

impl From<color::Model> for Color {
	fn from(color: color::Model) -> Self {
		Color {
			name: color.name,
			value: color.value as u32,
		}
	}
}

pub async fn replace_palette<Connection: TransactionTrait>(
	palette: &Palette,
	board_id: i32,
	connection: &Connection,
) -> DbResult<()> {
	let transaction = connection.begin().await?;

	let palette = palette.clone();
		
	color::Entity::delete_many()
		.filter(color::Column::Board.eq(board_id))
		.exec(&transaction).await?;

	for (index, Color { name, value }) in palette {
		let color = color::ActiveModel {
			board: Set(board_id),
			index: Set(index as i32),
			name: Set(name.clone()),
			value: Set(value as i32),
		};

		color::Entity::insert(color)
			.exec(&transaction).await?;
	}
	
	transaction.commit().await
}
