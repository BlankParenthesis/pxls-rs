use serde::{Serialize, Deserialize};
use rusqlite::{Result, Connection, params, Transaction};
use std::collections::HashMap;

pub type Palette = HashMap<usize, Color>;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Color {
	pub name: String,
	pub value: u32,
}

pub fn save_palette(
	palette: &Palette, 
	board_id: usize,
	connection: &mut Connection,
) -> Result<()> {
	let transaction = connection.transaction()?;
	
	save_palette_transaction(palette, board_id, &transaction)?;

	transaction.commit()?;

	Ok(())
}

pub fn save_palette_transaction(
	palette: &Palette, 
	board_id: usize,
	transaction: &Transaction,
) -> Result<()> {
	for (index, Color{ name, value }) in palette {
		transaction.execute(
			"INSERT INTO `color` VALUES(?1, ?2, ?3, ?4)",
			params![board_id, index, name, value],
		)?;
	}

	Ok(())
}