#[macro_use] extern crate lazy_static;

extern crate actix_web;
extern crate serde;
extern crate futures_util;

#[macro_use] mod access;
mod routes;

use actix_web::{App, HttpServer, middleware};

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    HttpServer::new(|| App::new()
        .wrap(middleware::NormalizePath::new(middleware::normalize::TrailingSlash::Trim))
        .service(routes::core::info::info)
        .service(routes::core::access::access)
        .service(routes::core::boards::list)
        .service(routes::core::boards::get_default)
        .service(routes::core::boards::get)
    ).bind("127.0.0.1:8000")?
        .run()
        .await
}