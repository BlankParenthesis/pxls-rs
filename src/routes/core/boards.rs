use std::sync::Arc;
use std::ops::Deref;

use tokio::sync::RwLockReadGuard;
use ouroboros::self_referencing;
use reqwest::StatusCode;
use serde::Deserialize;
use warp::http::Uri;
use warp::hyper::Response;
use warp::ws::Ws;
use warp::{Reply, Rejection};
use warp::{http::header, Filter};
use warp::path::Tail;

use crate::database::{BoardsDatabase, BoardsConnection, UsersDatabase};
use crate::filter::header::authorization::{Bearer, authorized};
use crate::filter::resource::{board::{self, PassableBoard}, database};
use crate::filter::response::reference::Reference;
use crate::permissions::Permission;
use crate::socket::{Extension, UnauthedSocket};
use crate::filter::response::paginated_list::*;
use crate::board::Board;

use crate::{BoardRef, BoardDataMap};

pub mod data;
pub mod pixels;

#[derive(Debug, Deserialize, Default)]
struct BoardPageToken(usize);

impl PageToken for BoardPageToken {
	fn start() -> Self {
		BoardPageToken(0)
	}
}

#[self_referencing]
struct OwnedBoardReference<'a> {
	board: &'a BoardRef,
	#[covariant]
	#[borrows(board)]
	lock: RwLockReadGuard<'this, Option<Board>>,
	#[borrows(lock)]
	inner: &'this Board,
}

impl<'a> Deref for OwnedBoardReference<'a> {
	type Target = Board;

	fn deref(&self) -> &Self::Target {
		self.borrow_inner()
	}
}

pub fn list(
	boards: BoardDataMap,
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("boards")
		.and(warp::path::end())
		.and(warp::get())
		.and(warp::query())
		.and(authorized(users_db, Permission::BoardsList.into()))
		.then(move |pagination: PaginationOptions<BoardPageToken>, _, _| {
			let boards = Arc::clone(&boards);
			async move {
				let page = pagination.page.0;
				let limit = pagination
					.limit
					.unwrap_or(DEFAULT_PAGE_ITEM_LIMIT)
					.clamp(1, MAX_PAGE_ITEM_LIMIT);

				let boards = boards.read().await;
				let boards = boards.iter()
					.map(|(_id, board)| board)
					.collect::<Vec<_>>();
				let mut pages = boards.chunks(limit)
					.skip(page.saturating_sub(2));
				
				fn page_uri(
					page: usize,
					limit: usize,
				) -> Uri {
					format!("/boards?page={}&limit={}", page, limit)
						.parse().unwrap()
				}

				let previous = page.checked_sub(1).and_then(|page| {
					pages.next().map(|_| page_uri(page, limit))
				});

				let mut items = Vec::with_capacity(limit);
				for board in pages.next().unwrap_or_default() {					
					let board = OwnedBoardReferenceAsyncSendBuilder {
						board,
						lock_builder: |board| Box::pin(async move {
							board.read().await
						}),
						inner_builder: |lock| Box::pin(async move {
							lock.as_ref().expect("Board went missing")
						}),
					}.build().await;
					items.push(board);
				}
				let items = items.iter()
					.map(|b| Reference::from(b.deref()))
					.collect();

				let next = page.checked_add(1).and_then(|page| {
					pages.next().map(|_| page_uri(page, limit))
				});

				let page = Page { previous, items, next };

				warp::reply::json(&page)
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
		.and(authorized(users_db, Permission::BoardsGet.into()))
		.and(database::connection(boards_db))
		.then(|board: PassableBoard, user, _, connection: BoardsConnection| async move {
			let board = board.read().await;
			let board = board.as_ref().expect("Board went missing when getting info");
			let mut response = warp::reply::json(&board.info).into_response();

			if let Some(Bearer { id, .. }) = user {
				let cooldown_info = board.user_cooldown_info(
					&id,
					&connection,
				).await;

				match cooldown_info {
					Ok(cooldown_info) => {
						for (key, value) in cooldown_info.into_headers() {
							response = warp::reply::with_header(response, key, value)
								.into_response();
						}
					},
					Err(err) => {
						// Indicate there was an error, but keep the body since
						// we have the data and are only missing the cooldown.
						response = warp::reply::with_status(
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
			options.extensions
				.ok_or(StatusCode::UNPROCESSABLE_ENTITY)
				.and_then(|extensions| {
					if extensions.is_empty() {
						return Err(StatusCode::UNPROCESSABLE_ENTITY);
					}

					if !extensions.contains(Extension::Authentication) {
						let permissions = Permission::defaults();
						let has_permissions = extensions.iter()
							.map(|e| e.socket_permission())
							.all(|p| permissions.contains(p));
					
						if !has_permissions {
							return Err(StatusCode::FORBIDDEN);
						}
					}
				
					let users_db = Arc::clone(&users_db);
					
					Ok(ws.on_upgrade(move |websocket| {
						UnauthedSocket::connect(
							websocket,
							extensions,
							Arc::downgrade(&*board),
							connection,
							users_db,
						)
					}))
				})
		})
		.recover(|rejection: Rejection| async {
			if let Some(err) = rejection.find::<serde_qs::Error>() {
				Ok(StatusCode::UNPROCESSABLE_ENTITY.into_response())
			} else {
				Err(rejection)
			}
		})
}
