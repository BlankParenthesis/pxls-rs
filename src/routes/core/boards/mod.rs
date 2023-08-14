use std::sync::Arc;
use std::ops::Deref;

use http::header;
use tokio::sync::RwLock;
use warp::path::Tail;
use sea_orm::DatabaseConnection as Connection;

use super::*;
use crate::{
	filters::resource::board::{PassableBoard, PendingDelete},
	objects::socket::Extension,
	BoardDataMap,
};

pub mod data;
pub mod pixels;
pub mod users;

pub fn list(boards: BoardDataMap) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("boards")
		.and(warp::path::end())
		.and(warp::get())
		.and(authorization::bearer().and_then(with_permission(Permission::BoardsList)))
		.and(warp::query())
		.then(move |_user, pagination: PaginationOptions<usize>| {
			let boards = Arc::clone(&boards);
			async move {
				let page = pagination.page.unwrap_or(0);
				let limit = pagination
					.limit
					.unwrap_or(10)
					.clamp(1, 100);

				let boards = Arc::clone(&boards);
				let boards = boards.read().await;
				let boards = boards.iter()
					.map(|(_id, board)| board)
					.collect::<Vec<_>>();
				let mut pages = boards.chunks(limit)
					.skip(page.saturating_sub(2));
				
				fn page_uri(
					page: usize,
					limit: usize,
				) -> String {
					format!("/boards?page={}&limit={}", page, limit)
				}

				let previous = page.checked_sub(1).and_then(|page| {
					pages.next().map(|_| page_uri(page, limit))
				});

				let mut items = Vec::with_capacity(limit);
				for board in pages.next().unwrap_or_default() {
					let board = board.read().await;
					let reference = Reference::from(board.deref().as_ref().unwrap());
					items.push(serde_json::to_value(reference).unwrap());
				}

				let next = page.checked_add(1).and_then(|page| {
					pages.next().map(|_| page_uri(page, limit))
				});

				// TODO: standardize generation of this
				let response = Page { previous, items: items.as_slice(), next };

				json(&response).into_response()
			}
		})
}

pub fn default() -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("boards")
		.and(warp::path("default"))
		.and(warp::path::tail())
		.map(|path_tail: Tail| {
			// TODO: determine which board to use as default.
			let id = 1;

			Response::builder()
				.status(StatusCode::SEE_OTHER)
				.header(
					header::LOCATION,
					format!("/boards/{}/{}", id, path_tail.as_str()),
				)
				.body("")
				.unwrap()
		})
}

pub fn get(
	boards: BoardDataMap,
	database_pool: Arc<Connection>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("boards")
		.and(board::path::read(&boards))
		.and(warp::path::end())
		.and(warp::get())
		.and(authorization::bearer().and_then(with_permission(Permission::BoardsGet)))
		.and(database::connection(database_pool))
		.then(|board: PassableBoard, user, connection: Arc<Connection>| async move {
			let board = board.read().await;
			let board = board.as_ref().unwrap();
			let mut response = json(&board.info).into_response();

			if let AuthedUser::Authed { user, .. } = user {
				let cooldown_info = board
					.user_cooldown_info(&user, connection.as_ref()).await
					.unwrap(); // TODO: bad unwrap?

				for (key, value) in cooldown_info.into_headers() {
					response = reply::with_header(response, key, value).into_response();
				}
			}

			response
		})
}

pub fn post(
	boards: BoardDataMap,
	database_pool: Arc<Connection>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("boards")
		.and(warp::path::end())
		.and(warp::post())
		.and(warp::body::json())
		.and(authorization::bearer().and_then(with_permission(Permission::BoardsPost)))
		.and(database::connection(database_pool))
		.then(move |data: BoardInfoPost, _user, connection: Arc<Connection>| {
			let boards = Arc::clone(&boards);
			async move {
				let board = Board::create(data, connection.as_ref()).await.unwrap(); // TODO: bad unwrap?
				let id = board.id as usize;

				let mut boards = boards.write().await;
				boards.insert(id, Arc::new(RwLock::new(Some(board))));

				let board = boards.get(&id).unwrap().read().await;
				let board = board.as_ref().unwrap();

				let mut response = json(&Reference::from(board)).into_response();
				response = reply::with_status(response, StatusCode::CREATED).into_response();
				response = reply::with_header(
					response,
					header::LOCATION,
					http::Uri::from(board).to_string(),
				)
				.into_response();
				response
			}
		})
}

pub fn patch(
	boards: BoardDataMap,
	database_pool: Arc<Connection>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("boards")
		.and(board::path::read(&boards))
		.and(warp::path::end())
		.and(warp::patch())
		// TODO: require application/merge-patch+json type?
		.and(warp::body::json())
		.and(authorization::bearer().and_then(with_permission(Permission::BoardsPatch)))
		.and(database::connection(database_pool))
		.then(|board: PassableBoard, patch: BoardInfoPatch, _user, connection: Arc<Connection>| async move {
			let mut board = board.write().await;
			let board = board.as_mut().unwrap();

			board.update_info(patch, connection.as_ref()).await.unwrap(); // TODO: bad unwrap?

			let mut response = json(&Reference::from(&*board)).into_response();
			response = reply::with_status(response, StatusCode::CREATED).into_response();
			response = reply::with_header(response, header::LOCATION, http::Uri::from(&*board).to_string()).into_response();
			response
		})
}

pub fn delete(
	boards: BoardDataMap,
	database_pool: Arc<Connection>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("boards")
		.and(board::path::prepare_delete(&boards))
		.and(warp::path::end())
		.and(warp::delete())
		.and(authorization::bearer().and_then(with_permission(Permission::BoardsDelete)))
		.and(database::connection(database_pool))
		.then(move |mut deletion: PendingDelete, _user: AuthedUser, connection: Arc<Connection>| async move {
			let board = deletion.perform();
			let mut board = board.write().await;
			let board = board.take().unwrap();
			board.delete(connection.as_ref()).await.unwrap(); // TODO: bad unwrap?
			StatusCode::NO_CONTENT.into_response()
		})
}

#[derive(serde::Deserialize)]
pub struct SocketOptions {
	pub extensions: Option<enumset::EnumSet<Extension>>,
}

pub fn socket(
	boards: BoardDataMap,
	database_pool: Arc<Connection>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("boards")
		.and(board::path::read(&boards))
		.and(warp::path("socket"))
		.and(warp::path::end())
		.and(serde_qs::warp::query(Default::default()))
		.and(warp::ws())
		.map(
			move |board: PassableBoard, options: SocketOptions, ws: warp::ws::Ws| {
				let database_pool = Arc::clone(&database_pool);

				if let Some(extensions) = options.extensions {
					if !extensions.is_empty() {
						ws.on_upgrade(move |websocket| {
							UnauthedSocket::connect(
								websocket,
								extensions,
								Arc::downgrade(&*board),
								database_pool,
							)
						})
						.into_response()
					} else {
						StatusCode::UNPROCESSABLE_ENTITY.into_response()
					}
				} else {
					StatusCode::UNPROCESSABLE_ENTITY.into_response()
				}
			},
		)
		.recover(|rejection: Rejection| {
			async {
				if let Some(err) = rejection.find::<serde_qs::Error>() {
					Ok(StatusCode::UNPROCESSABLE_ENTITY.into_response())
				} else {
					Err(rejection)
				}
			}
		})
}
