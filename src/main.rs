#[macro_use] extern crate lazy_static;

#[macro_use] mod access;
mod routes;
mod socket;
mod objects;

use actix::prelude::*;
use actix_web::{App, HttpServer, middleware, web::Data};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::socket::server::BoardServer;
use crate::objects::board::Board;
use crate::objects::color::Color;

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    let board_server = Data::new(BoardServer::default().start());
    let board = Data::new(Board::new(
		String::from("c0"),
		SystemTime::now()
			.duration_since(UNIX_EPOCH).unwrap()
			.as_secs() as u64,
		[[1000, 1000]],
		vec![Color { name: String::from("red"), value: 0xff0000 }],
    ));

	HttpServer::new(move || App::new()
		.app_data(board.clone())
		.app_data(board_server.clone())
		.wrap(middleware::NormalizePath::new(middleware::normalize::TrailingSlash::Trim))
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