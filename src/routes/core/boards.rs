use std::sync::Arc;
use std::ops::Deref;

use reqwest::StatusCode;
use warp::hyper::Response;
use warp::reply::{json, self};
use warp::ws::Ws;
use warp::{Reply, Rejection};
use warp::{http::header, Filter};
use warp::path::Tail;

use crate::database::{BoardsDatabase, BoardsConnection, UsersDatabase};
use crate::filter::header::authorization::{Bearer, authorized};
use crate::filter::resource::{board, database};
use crate::{
	permissions::Permission,
	filter::resource::board::PassableBoard,
	socket::{Extension, UnauthedSocket},
	BoardDataMap,
	filter::response::reference::Reference,
	filter::response::paginated_list::*,
};

pub mod data;
pub mod pixels;
pub mod users;

pub fn list(
	boards: BoardDataMap,
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("boards")
		.and(warp::path::end())
		.and(warp::get())
		.and(warp::query())
		.and(authorized(users_db, &[Permission::BoardsList]))
		.then(move |pagination: PaginationOptions<usize>, _, _| {
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
					let board = board.deref().as_ref().expect("Board went missing during listing");
					let reference = Reference::from(board);
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
	boards_db: Arc<BoardsDatabase>,
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("boards")
		.and(board::path::read(&boards))
		.and(warp::path::end())
		.and(warp::get())
		.and(authorized(users_db, &[Permission::BoardsGet]))
		.and(database::connection(boards_db))
		.then(|board: PassableBoard, user, _, connection: BoardsConnection| async move {
			let board = board.read().await;
			let board = board.as_ref().expect("Board wend missing when getting info");
			let mut response = json(&board.info).into_response();

			if let Some(Bearer { id, .. }) = user {
				let cooldown_info = board.user_cooldown_info(
					&id,
					&connection,
				).await;

				match cooldown_info {
					Ok(cooldown_info) => {
						for (key, value) in cooldown_info.into_headers() {
							response = reply::with_header(response, key, value)
								.into_response();
						}
					},
					Err(err) => {
						// Indicate there was an error, but keep the body since
						// we have the data and are only missing the cooldown.
						response = reply::with_status(
							response,
							StatusCode::INTERNAL_SERVER_ERROR,
						).into_response()
					},
				}
			}

			response
		})
}

#[derive(serde::Deserialize)]
pub struct SocketOptions {
	pub extensions: Option<enumset::EnumSet<Extension>>,
}

pub fn socket(
	boards: BoardDataMap,
	boards_db: Arc<BoardsDatabase>,
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("boards")
		.and(board::path::read(&boards))
		.and(warp::path("socket"))
		.and(warp::path::end())
		.and(warp::ws())
		.and(serde_qs::warp::query(Default::default()))
		.and(database::connection(boards_db))
		.map(move |board: PassableBoard, ws: Ws, options: SocketOptions, connection: BoardsConnection| {
			if let Some(extensions) = options.extensions {
				if extensions.is_empty() {
					return StatusCode::UNPROCESSABLE_ENTITY.into_response();
				}

				if !extensions.contains(Extension::Authentication) {
					let permissions = Permission::defaults();
					let has_permissions = extensions.iter()
						.map(|e| e.socket_permission())
						.all(|p| permissions.contains(p));
				
					if !has_permissions {
						return StatusCode::FORBIDDEN.into_response();
					}
				}
			
				let users_db = Arc::clone(&users_db);
				ws.on_upgrade(move |websocket| {
					UnauthedSocket::connect(
						websocket,
						extensions,
						Arc::downgrade(&*board),
						connection,
						users_db,
					)
				})
				.into_response()
			} else {
				StatusCode::UNPROCESSABLE_ENTITY.into_response()
			}
		})
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
