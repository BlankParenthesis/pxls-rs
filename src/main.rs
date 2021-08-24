#[macro_use] extern crate lazy_static;

#[macro_use] mod access;
mod routes;
mod socket;
mod objects;
mod database;

use actix::prelude::*;
use actix_web::{App, HttpServer, web::Data};
use actix_web::middleware::{NormalizePath, normalize::TrailingSlash};

use crate::socket::server::BoardServer;

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    let board_server = Data::new(BoardServer::default().start());

	let manager = r2d2_sqlite::SqliteConnectionManager::file("pxls.db");
	let pool = r2d2::Pool::new(manager).unwrap();

	let connection = pool.get().expect("Could not connected to database");
	database::queries::init(connection).expect("Could not init database");

	let connection = pool.get().expect("Could not connected to database");
    let boards = Data::new(database::queries::load_boards(connection)
		.expect("Failed to load boards"));

	HttpServer::new(move || App::new()
		.data(pool.clone())
		.app_data(boards.clone())
		.app_data(board_server.clone())
		.wrap(NormalizePath::new(TrailingSlash::Trim))
		.service(routes::core::info::info)
		.service(routes::core::access::access)
		.service(routes::core::boards::list)
		.service(routes::core::boards::get_default)
		.service(routes::core::boards::get)
		.service(routes::core::boards::socket)
		.service(routes::core::boards::get_color_data)
		.service(routes::core::boards::get_timestamp_data)
		.service(routes::core::boards::get_mask_data)
		.service(routes::core::boards::get_users)
	).bind("127.0.0.1:8000")?
		.run()
		.await
}