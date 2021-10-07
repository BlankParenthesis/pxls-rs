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
use std::sync::RwLock;
use actix_web::{App, HttpServer, web::Data};
use actix_web::middleware::{NormalizePath, normalize::TrailingSlash, Compress};

use crate::objects::Board;

pub type BoardDataMap = Data<RwLock<HashMap<usize, RwLock<Board>>>>;

embed_migrations!();

#[actix_web::main]
async fn main() -> std::io::Result<()> {
	let config = crate::config::CONFIG.read().unwrap();

	let manager = diesel::r2d2::ConnectionManager::new(config.database_url.to_string());
	let pool = r2d2::Pool::new(manager).unwrap();
	let connection = pool.get().unwrap();

	embedded_migrations::run_with_output(&connection, &mut std::io::stdout())
		.expect("Migration failed");

	let connection = pool.get().unwrap();
	let boards = database::queries::load_boards(&connection)
		.expect("Failed to load boards")
		.into_iter()
		.map(|board| (board.id as usize, RwLock::new(board)))
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
		.wrap(Compress::default())
		.service(routes::core::info::get)
		.service(routes::core::access::get)
		.service(routes::core::boards::list)
		.service(routes::core::boards::get)
		.service(routes::core::boards::get_default)
		.service(routes::core::boards::post)
		.service(routes::core::boards::patch)
		.service(routes::core::boards::delete)
		.service(routes::core::boards::socket)
		.service(routes::core::boards::data::get_colors)
		.service(routes::core::boards::data::get_timestamps)
		.service(routes::core::boards::data::get_mask)
		.service(routes::core::boards::data::get_initial)
		.service(routes::core::boards::data::patch_initial)
		.service(routes::core::boards::data::patch_mask)
		.service(routes::core::boards::users::get)
		.service(routes::core::boards::pixels::list)
		.service(routes::core::boards::pixels::get)
		.service(routes::core::boards::pixels::post)
		.service(routes::auth::auth::get)
	).bind(format!("{}:{}", config.host, config.port))?
		.run()
		.await
}