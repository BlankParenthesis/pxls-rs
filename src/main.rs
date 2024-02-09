#[macro_use]
extern crate lazy_static;

#[macro_use]
mod database;
mod openid;
mod config;
mod filter;
mod board;
mod routes;
mod socket;
mod permissions;

use std::time::Duration;
use std::{collections::HashMap, sync::Arc};

use deadpool::managed::Pool;
use sea_orm::{Database, ConnectOptions};
use filter::header::authorization::{BearerError, PermissionsError};
use futures_util::future;
use tokio::sync::RwLock;
use warp::{Filter, Rejection, Reply};
use warp::http::{Method, StatusCode};

use crate::database::boards::migrations::{Migrator, MigratorTrait};
use crate::database::boards::DatabaseError;
use crate::database::users::LDAPConnectionManager;
use crate::board::Board;
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

#[tokio::main]
async fn main() {
	let mut connect_options = ConnectOptions::new(CONFIG.database_url.to_string());
	connect_options
		.connect_timeout(Duration::from_secs(2))
		.acquire_timeout(Duration::from_secs(2));

	let db = Arc::new(Database::connect(connect_options).await
		.expect("Failed to connect to database"));

	Migrator::up(db.as_ref(), None).await.expect("Failed to run migrations");

	let boards = database::query::load_boards(db.as_ref())
		.await
		.expect("Failed to load boards")
		.into_iter()
		.map(|board| (board.id as usize, Arc::new(RwLock::new(Some(board)))))
		.collect::<HashMap<_, _>>();

	let boards: BoardDataMap = Arc::new(RwLock::new(boards));

	let ldap_url = String::from(CONFIG.users_ldap_url.as_str());
	let users_db_manager = LDAPConnectionManager(ldap_url);
	let users_db_pool = Pool::<LDAPConnectionManager>::builder(users_db_manager)
		.build()
		.expect("Failed to start LDAP connection pool");
	let users_db_pool = Arc::new(users_db_pool);

	let routes = routes::core::info::get()
		.or(routes::core::access::get())
		.or(routes::core::boards::list(Arc::clone(&boards)))
		.or(routes::core::boards::get(
			Arc::clone(&boards),
			Arc::clone(&db),
		))
		.or(routes::core::boards::default())
		.or(routes::board_lifecycle::boards::post(
			Arc::clone(&boards),
			Arc::clone(&db),
		))
		.or(routes::board_lifecycle::boards::patch(
			Arc::clone(&boards),
			Arc::clone(&db),
		))
		.or(routes::board_lifecycle::boards::delete(
			Arc::clone(&boards),
			Arc::clone(&db),
		))
		.or(routes::core::boards::socket(
			Arc::clone(&boards),
			Arc::clone(&db),
		))
		.or(routes::core::boards::data::get_colors(
			Arc::clone(&boards),
			Arc::clone(&db),
		))
		.or(routes::board_data_initial::boards::data::get_initial(
			Arc::clone(&boards),
			Arc::clone(&db),
		))
		.or(routes::board_data_mask::boards::data::get_mask(
			Arc::clone(&boards),
			Arc::clone(&db),
		))
		.or(routes::board_data_timestamps::boards::data::get_timestamps(
			Arc::clone(&boards),
			Arc::clone(&db),
		))
		.or(routes::board_data_initial::boards::data::patch_initial(
			Arc::clone(&boards),
			Arc::clone(&db),
		))
		.or(routes::board_data_mask::boards::data::patch_mask(
			Arc::clone(&boards),
			Arc::clone(&db),
		))
		.or(routes::core::boards::users::get(
			Arc::clone(&boards),
			Arc::clone(&db),
		))
		.or(routes::core::boards::pixels::list(
			Arc::clone(&boards),
			Arc::clone(&db),
		))
		.or(routes::core::boards::pixels::get(
			Arc::clone(&boards),
			Arc::clone(&db),
		))
		.or(routes::core::boards::pixels::post(
			Arc::clone(&boards),
			Arc::clone(&db),
		))
		.or(routes::authentication::authentication::get())
		.or(routes::users::users::list(&users_db_pool))
		.or(routes::users::users::current())
		.or(routes::users::users::get(&users_db_pool))
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
	// Update: still waiting… progress doesn't look good
	let gzip_routes = filter::header::accept_encoding::gzip()
		.and(routes.clone())
		.with(warp::compression::gzip());

	let binding = ([0, 0, 0, 0], CONFIG.port);
	let exit_signal = async {
		tokio::signal::ctrl_c().await.expect("ctrl+c interrupt error");
	};

	let (_, server) = warp::serve(gzip_routes.or(routes))
		.bind_with_graceful_shutdown(binding, exit_signal);

	server.await
}

// TODO: move this elsewhere
// it's pretty general, but I'm hesitatn to create a util/misc module just yet

use async_trait::async_trait;

#[async_trait]
pub trait AsyncRead {
	type Error;

	async fn read(
		&mut self,
		output: &mut [u8],
	) -> std::result::Result<usize, Self::Error>;
}

#[async_trait]
pub trait AsyncWrite {
	type Error;

	async fn write(
		&mut self,
		input: &[u8],
	) -> std::result::Result<usize, Self::Error>;

	async fn flush(&mut self) -> std::result::Result<(), Self::Error>;
}

pub trait Len {
	fn len(&self) -> usize;

	fn is_empty(&self) -> bool {
		self.len() == 0
	}
}