use rusqlite::Result;

use crate::objects::Board;

pub type Connection = r2d2::PooledConnection<r2d2_sqlite::SqliteConnectionManager>;

pub trait DatabaseStorable {
	fn save(connection: &Connection) -> Result<()>;
}

pub trait DatabaseLoadable {
	fn load(id: usize, connection: &Connection) -> Result<Option<Self>>
	where Self: std::marker::Sized;
}

pub fn init(connection: Connection) -> Result<()> {
	connection.execute(include_str!("sql/setup.sql"), [])?;
	connection.execute(include_str!("sql/create_board_table.sql"), [])?;
	connection.execute(include_str!("sql/create_color_table.sql"), [])?;
	connection.execute(include_str!("sql/create_placement_table.sql"), [])?;

	Ok(())
}

pub fn load_boards(connection: &Connection) -> Result<Vec<Board>> {
	connection.prepare("SELECT `id` FROM `board`")?
		.query_map([], |board| Ok(Board::load(
			board.get(0)?, 
			connection,
		)?.unwrap()))?
		.collect()
}