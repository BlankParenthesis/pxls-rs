pub mod queries;

pub fn open_database(pool: &queries::Pool) -> queries::Connection {
	let connection = pool.get().expect("pool");
	connection.execute(include_str!("sql/setup.sql"), [])
		.expect("setup failed");
	connection
}
