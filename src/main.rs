#![recursion_limit = "256"]

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

use std::{collections::HashMap, sync::Arc};

use database::{BoardsDatabase, UsersDatabase, Database, UsersDatabaseError};
use filter::header::authorization::{BearerError, PermissionsError};
use futures_util::future;
use routes::core::Connections;
use tokio::sync::RwLock;
use warp::body::BodyDeserializeError;
use warp::{Filter, Rejection, Reply};
use warp::http::{Method, StatusCode};

use crate::board::Board;
use crate::config::CONFIG;

// It seems like it would be nice if this were just RwLock<Board>.
// This cannot be done because we pass the board into the delete function to
// dispose of the object clearly. To do that, we need to own the board fully.
// with RwLock, we cannot take the board out of that reference (even if we
// have a write lock) unless we own the RwLock. We don't own the RwLock since
// it's shared behind Arc. With Option we can take the board and replace it
// with None if we have a &mut through the RwLock.
type BoardRef = Arc<RwLock<Option<Board>>>;
pub type BoardDataMap = Arc<RwLock<HashMap<usize, BoardRef>>>;

#[tokio::main]
async fn main() {
	let boards_db = BoardsDatabase::connect().await
		.expect("Failed to connect to boards database");
	let boards_db = Arc::new(boards_db);
	
	let users_db = UsersDatabase::connect().await
		.expect("Failed to connect to users database");
	let users_db = Arc::new(users_db);

	let connection = boards_db.connection().await
		.expect("Failed to get board connection when loading boards");
	let boards = connection
		.list_boards().await.expect("Failed to load boards (at list)")
		.into_iter()
		.map(|board| (board.id as usize, Arc::new(RwLock::new(Some(board)))))
		.collect::<HashMap<_, _>>();

	let boards: BoardDataMap = Arc::new(RwLock::new(boards));
	let sockets = Arc::new(RwLock::new(Connections::default()));

	let routes_core = 
		routes::core::info::get(Arc::clone(&users_db)).boxed()
		.or(routes::core::events(Arc::clone(&sockets), Arc::clone(&users_db)).boxed())
		.or(routes::core::access::get(Arc::clone(&users_db)).boxed())
		.or(routes::core::boards::list(Arc::clone(&boards), Arc::clone(&users_db)).boxed())
		.or(routes::core::boards::get(
			Arc::clone(&boards),
			Arc::clone(&boards_db),
			Arc::clone(&users_db),
		)).boxed()
		.or(routes::core::boards::default()).boxed()
		.or(routes::core::boards::events(
			Arc::clone(&boards),
			Arc::clone(&boards_db),
			Arc::clone(&users_db),
		)).boxed()
		.or(routes::core::boards::data::get_colors(
			Arc::clone(&boards),
			Arc::clone(&boards_db),
			Arc::clone(&users_db),
		)).boxed()
		.or(routes::core::boards::pixels::list(
			Arc::clone(&boards),
			Arc::clone(&boards_db),
			Arc::clone(&users_db),
		)).boxed()
		.or(routes::core::boards::pixels::get(
			Arc::clone(&boards),
			Arc::clone(&boards_db),
			Arc::clone(&users_db),
		)).boxed()
		.or(routes::core::boards::pixels::post(
			Arc::clone(&boards),
			Arc::clone(&boards_db),
			Arc::clone(&users_db),
		)).boxed();

	let routes_lifecycle = 
		routes::board_lifecycle::boards::post(
			Arc::clone(&sockets),
			Arc::clone(&boards),
			Arc::clone(&boards_db),
			Arc::clone(&users_db),
		).boxed()
		.or(routes::board_lifecycle::boards::patch(
			Arc::clone(&boards),
			Arc::clone(&boards_db),
			Arc::clone(&users_db),
		)).boxed()
		.or(routes::board_lifecycle::boards::delete(
			Arc::clone(&sockets),
			Arc::clone(&boards),
			Arc::clone(&boards_db),
			Arc::clone(&users_db),
		)).boxed();

	let routes_data_initial = 
		routes::board_data_initial::boards::data::get_initial(
			Arc::clone(&boards),
			Arc::clone(&boards_db),
			Arc::clone(&users_db),
		).boxed()
		.or(routes::board_data_initial::boards::data::patch_initial(
			Arc::clone(&boards),
			Arc::clone(&boards_db),
			Arc::clone(&users_db),
		)).boxed();

	let routes_data_mask = 
		routes::board_data_mask::boards::data::get_mask(
			Arc::clone(&boards),
			Arc::clone(&boards_db),
			Arc::clone(&users_db),
		).boxed()
		.or(routes::board_data_mask::boards::data::patch_mask(
			Arc::clone(&boards),
			Arc::clone(&boards_db),
			Arc::clone(&users_db),
		)).boxed();

	let routes_data_timestamps = 
		routes::board_data_timestamps::boards::data::get_timestamps(
			Arc::clone(&boards),
			Arc::clone(&boards_db),
			Arc::clone(&users_db),
		).boxed();

	let routes_authentication = 
		routes::authentication::authentication::get().boxed();

	let routes_users = 
		routes::users::users::list(Arc::clone(&users_db)).boxed()
		.or(routes::users::users::current(Arc::clone(&users_db)).boxed())
		.or(routes::users::users::get(Arc::clone(&users_db)).boxed())
		.or(routes::users::users::patch(Arc::clone(&users_db)).boxed())
		.or(routes::users::users::delete(Arc::clone(&users_db)).boxed());
		
	let routes_roles =
		routes::roles::users::roles::list(Arc::clone(&users_db)).boxed()
		.or(routes::roles::users::roles::post(Arc::clone(&sockets), Arc::clone(&users_db)).boxed())
		.or(routes::roles::users::roles::delete(Arc::clone(&sockets), Arc::clone(&users_db)).boxed())
		.or(routes::roles::roles::list(Arc::clone(&users_db)).boxed())
		.or(routes::roles::roles::get(Arc::clone(&users_db)).boxed())
		.or(routes::roles::roles::post(Arc::clone(&sockets), Arc::clone(&users_db)).boxed())
		.or(routes::roles::roles::patch(Arc::clone(&sockets), Arc::clone(&users_db)).boxed())
		.or(routes::roles::roles::delete(Arc::clone(&sockets), Arc::clone(&users_db)).boxed());
	
	let routes_usercount = 
		routes::user_count::boards::users(
			Arc::clone(&boards),
			Arc::clone(&boards_db),
			Arc::clone(&users_db),
		).boxed();

	let routes_board_moderation =
		routes::board_moderation::boards::pixels::patch(
			Arc::clone(&boards),
			Arc::clone(&boards_db),
			Arc::clone(&users_db),
		).boxed();

	let routes_undo = 
		routes::board_undo::boards::pixels::delete(
			Arc::clone(&boards),
			Arc::clone(&boards_db),
			Arc::clone(&users_db),
		).boxed();

	let routes_factions =
		routes::factions::factions::list(Arc::clone(&users_db)).boxed()
		.or(routes::factions::factions::get(Arc::clone(&users_db)).boxed())
		.or(routes::factions::factions::post(Arc::clone(&users_db)).boxed())
		.or(routes::factions::factions::patch(Arc::clone(&users_db)).boxed())
		.or(routes::factions::factions::delete(Arc::clone(&users_db)).boxed())
		.or(routes::factions::factions::members::list(Arc::clone(&users_db)).boxed())
		.or(routes::factions::factions::members::get(Arc::clone(&users_db)).boxed())
		.or(routes::factions::factions::members::post(Arc::clone(&users_db)).boxed())
		.or(routes::factions::factions::members::patch(Arc::clone(&users_db)).boxed())
		.or(routes::factions::factions::members::delete(Arc::clone(&users_db)).boxed());

	let routes_site_notices = 
		routes::site_notices::notices::list(Arc::clone(&boards_db), Arc::clone(&users_db)).boxed()
		.or(routes::site_notices::notices::get(Arc::clone(&boards_db), Arc::clone(&users_db)).boxed())
		.or(routes::site_notices::notices::post(Arc::clone(&boards_db), Arc::clone(&users_db)).boxed())
		.or(routes::site_notices::notices::patch(Arc::clone(&boards_db), Arc::clone(&users_db)).boxed())
		.or(routes::site_notices::notices::delete(Arc::clone(&boards_db), Arc::clone(&users_db)).boxed());

	let routes = 
		routes_core
		.or(routes_lifecycle)
		.or(routes_data_initial)
		.or(routes_data_mask)
		.or(routes_data_timestamps)
		.or(routes_authentication)
		.or(routes_users)
		.or(routes_roles)
		.or(routes_usercount)
		.or(routes_board_moderation)
		.or(routes_undo)
		.or(routes_factions)
		.or(routes_site_notices)
		.recover(|rejection: Rejection| {
			if let Some(err) = rejection.find::<BearerError>() {
				future::ok(StatusCode::UNAUTHORIZED.into_response())
			} else if let Some(err) = rejection.find::<PermissionsError>() {
				future::ok(StatusCode::FORBIDDEN.into_response())
			} else if let Some(err) = rejection.find::<BodyDeserializeError>() {
				future::ok(StatusCode::BAD_REQUEST.into_response())
			} else if let Some(err) = rejection.find::<UsersDatabaseError>() {
				future::ok(err.into_response())
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
// it's pretty general, but I'm hesitant to create a util/misc module just yet

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