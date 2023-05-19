#[macro_use]
extern crate lazy_static;
#[macro_use]
extern crate diesel;
#[macro_use]
extern crate diesel_migrations;

#[macro_use]
mod access;
#[macro_use]
mod database;
mod authentication;
mod config;
mod filters;
mod objects;
mod routes;
//mod socket;

use std::{collections::HashMap, sync::Arc};

use access::permissions::PermissionsError;
use filters::header::authorization::BearerError;
use futures_util::future;
use http::{Method, StatusCode};
//use tokio::sync::RwLock;
use parking_lot::RwLock;
use warp::{Filter, Rejection, Reply};

use crate::objects::Board;
use crate::config::CONFIG;

// FIXME: since we're not longer using actix, this is probably solvable?
// NOTE: This can go back to being RwLock<Board> if we can get nice ownership
// between the Board, BoardServer, and BoardServerSocket. Actix makes this
// impossible.
// The reason for this is that if BoardServer is owned by Board, Board *must*
// outlive it. This means that we can add a lifetime parameter to it and give it
// a board reference directly, rather than resorting to reference counted
// maybe-there, maybe-not solutions (like below).
type BoardRef = Arc<RwLock<Option<Board>>>;
pub type BoardDataMap = Arc<RwLock<HashMap<usize, BoardRef>>>;

embed_migrations!();

#[tokio::main]
async fn main() {
	let manager = diesel::r2d2::ConnectionManager::new(CONFIG.database_url.to_string());
	let pool = Arc::new(r2d2::Pool::new(manager).unwrap());
	let connection = pool.get().unwrap();

	embedded_migrations::run_with_output(&connection, &mut std::io::stdout())
		.expect("Migration failed");

	let connection = pool.get().unwrap();
	let boards = database::queries::load_boards(&connection)
		.expect("Failed to load boards")
		.into_iter()
		.map(|board| (board.id as usize, Arc::new(RwLock::new(Some(board)))))
		.collect::<HashMap<_, _>>();

	let boards: BoardDataMap = Arc::new(RwLock::new(boards));

	let routes = routes::core::info::get()
		.or(routes::core::access::get())
		.or(routes::core::boards::list(Arc::clone(&boards)))
		.or(routes::core::boards::get(
			Arc::clone(&boards),
			Arc::clone(&pool),
		))
		.or(routes::core::boards::default())
		.or(routes::core::boards::post(
			Arc::clone(&boards),
			Arc::clone(&pool),
		))
		.or(routes::core::boards::patch(
			Arc::clone(&boards),
			Arc::clone(&pool),
		))
		.or(routes::core::boards::delete(
			Arc::clone(&boards),
			Arc::clone(&pool),
		))
		.or(routes::core::boards::socket(
			Arc::clone(&boards),
			Arc::clone(&pool),
		))
		.or(routes::core::boards::data::get_colors(
			Arc::clone(&boards),
			Arc::clone(&pool),
		))
		.or(routes::core::boards::data::get_initial(
			Arc::clone(&boards),
			Arc::clone(&pool),
		))
		.or(routes::core::boards::data::get_mask(
			Arc::clone(&boards),
			Arc::clone(&pool),
		))
		.or(routes::core::boards::data::get_timestamps(
			Arc::clone(&boards),
			Arc::clone(&pool),
		))
		.or(routes::core::boards::data::patch_initial(
			Arc::clone(&boards),
			Arc::clone(&pool),
		))
		.or(routes::core::boards::data::patch_mask(
			Arc::clone(&boards),
			Arc::clone(&pool),
		))
		.or(routes::core::boards::users::get(
			Arc::clone(&boards),
			Arc::clone(&pool),
		))
		.or(routes::core::boards::pixels::list(
			Arc::clone(&boards),
			Arc::clone(&pool),
		))
		.or(routes::core::boards::pixels::get(
			Arc::clone(&boards),
			Arc::clone(&pool),
		))
		.or(routes::core::boards::pixels::post(
			Arc::clone(&boards),
			Arc::clone(&pool),
		))
		.or(routes::auth::auth::get())
		.recover(|rejection: Rejection| {
			if let Some(err) = rejection.find::<BearerError>() {
				future::ok(StatusCode::UNAUTHORIZED.into_response())
			} else if let Some(err) = rejection.find::<PermissionsError>() {
				future::ok(StatusCode::FORBIDDEN.into_response())
			} else {
				future::err(rejection)
			}
		})
		.with(
			warp::cors::cors()
				.allow_any_origin()
				.allow_credentials(true)
				.allow_methods([
					Method::GET,
					Method::POST,
					Method::DELETE,
					Method::PATCH,
				]), // TODO: allow headers
		);

	// Temporary fix for gzip until https://github.com/seanmonstar/warp/pull/513
	// is merged
	let gzip_routes = filters::header::accept_encoding::gzip()
		.and(routes.clone())
		.with(warp::compression::gzip());

	warp::serve(gzip_routes.or(routes))
		.run(([127, 0, 0, 1], CONFIG.port))
		.await;
}
