use serde::{Serialize, Deserialize};
use std::collections::HashMap;
use diesel::prelude::*;
use diesel::Connection as DConnection;

use crate::database::{Connection, model, schema};

pub type Palette = HashMap<u32, Color>;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Color {
	pub name: String,
	pub value: u32,
}

impl From<model::Color> for Color {
	fn from(color: model::Color) -> Self {
		Color { 
			name: color.name,
			value: color.value as u32,
		}
	}
}

pub fn replace_palette(
	palette: &Palette, 
	board_id: i32,
	connection: &Connection,
) -> QueryResult<()> {
	connection.transaction(|| {
		diesel::delete(schema::color::table)
			.filter(schema::color::board.eq(board_id))
			.execute(connection)?;

		for (index, Color{ name, value }) in palette {
			diesel::insert_into(schema::color::table)
				.values(model::Color {
					board: board_id,
					index: *index as i32,
					name: name.clone(),
					value: *value as i32,
				})
				.execute(connection)?;
		}
		Ok(())
	})
}