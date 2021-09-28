#[macro_use] extern crate lazy_static;
#[macro_use] extern crate diesel;
#[macro_use] extern crate diesel_migrations;

#[macro_use] mod access;
#[macro_use] mod database;
mod routes;
mod socket;
mod objects;
mod config;

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use actix::prelude::*;
use actix_web::{App, HttpServer, web::Data};
use actix_web::middleware::{NormalizePath, normalize::TrailingSlash};

use crate::objects::Board;
use crate::socket::server::BoardServer;

pub struct BoardData(RwLock<Board>, Arc<Addr<BoardServer>>);
pub type BoardDataMap = Data<RwLock<HashMap<usize, BoardData>>>;

impl BoardData {
	fn new(board: Board) -> Self {
		Self(RwLock::new(board), Arc::new(BoardServer::default().start()))
	}
}

embed_migrations!();

#[actix_web::main]
async fn main() -> std::io::Result<()> {
	let config = crate::config::CONFIG.try_read().unwrap();

	let manager = diesel::r2d2::ConnectionManager::new(config.database_url.to_string());
	let pool = r2d2::Pool::new(manager).unwrap();
	let connection = pool.get().unwrap();

    embedded_migrations::run_with_output(&connection, &mut std::io::stdout())
		.expect("Migration failed");

	let connection = pool.get().unwrap();
	let boards = database::queries::load_boards(&connection)
		.expect("Failed to load boards")
		.into_iter()
		.map(|board| (board.id as usize, BoardData::new(board)))
		.collect::<HashMap<_, _>>();
	let boards: BoardDataMap = Data::new(RwLock::new(boards));

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
		.service(routes::core::boards::get_pixels)
		.service(routes::core::boards::get_pixel)
		.service(routes::core::boards::post_pixel)
		.service(routes::auth::auth::auth)
	).bind(format!("{}:{}", config.host, config.port))?
		.run()
		.await
}