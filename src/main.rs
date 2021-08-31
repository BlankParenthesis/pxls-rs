#[macro_use] extern crate lazy_static;

#[macro_use] mod access;
mod routes;
mod socket;
mod objects;
mod database;

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use actix::prelude::*;
use actix_web::{App, HttpServer, web::Data};
use actix_web::middleware::{NormalizePath, normalize::TrailingSlash};

use crate::objects::Board;
use crate::socket::server::BoardServer;

pub struct BoardData(RwLock<Board>, Arc<Addr<BoardServer>>);

impl BoardData {
	fn new(board: Board) -> Self {
		Self(RwLock::new(board), Arc::new(BoardServer::default().start()))
	}
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
	let manager = r2d2_sqlite::SqliteConnectionManager::file("pxls.db");
	let pool = r2d2::Pool::new(manager).unwrap();

	let connection = pool.get().expect("Could not connected to database");
	database::queries::init(connection).expect("Could not init database");

	let connection = pool.get().expect("Could not connected to database");
	let boards = database::queries::load_boards(&connection)
		.expect("Failed to load boards")
		.into_iter()
		.map(|board| (board.id, BoardData::new(board)))
		.collect::<HashMap<_, _>>();
	let boards = Data::new(RwLock::new(boards));

	HttpServer::new(move || App::new()
		.data(pool.clone())
		.app_data(boards.clone())
		.wrap(actix_cors::Cors::default()
			.allow_any_origin()
			.allow_any_header()
			.allow_any_method())
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
		.service(routes::core::boards::get_initial_data)
		.service(routes::core::boards::get_users)
		.service(routes::core::boards::post)
		.service(routes::core::boards::patch)
		.service(routes::core::boards::delete)
	).bind("127.0.0.1:8000")?
		.run()
		.await
}