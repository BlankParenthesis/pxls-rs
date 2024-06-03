use std::sync::Arc;
use std::ops::Deref;

use enumset::EnumSet;
use tokio::sync::RwLockReadGuard;
use ouroboros::self_referencing;
use reqwest::StatusCode;
use serde::Deserialize;
use url::form_urlencoded::byte_serialize;
use warp::http::Uri;
use warp::hyper::Response;
use warp::ws::Ws;
use warp::{Reply, Rejection};
use warp::{http::header, Filter};
use warp::path::Tail;

use crate::database::{BoardsDatabase, BoardsConnection, UsersDatabase};
use crate::filter::header::authorization::{Bearer, authorized};
use crate::filter::resource::filter::FilterRange;
use crate::filter::resource::{board::{self, PassableBoard}, database};
use crate::filter::response::reference::Reference;
use crate::permissions::Permission;
use crate::socket::{Socket, CloseReason};
use crate::filter::response::paginated_list::*;
use crate::board::{Board, BoardSubscription};

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

#[derive(Debug, Deserialize)]
struct BoardFilter {
	name: Option<String>,
	#[serde(default)]
	created_at: FilterRange<u64>,
	// TODO: maybe allow useful filtering of arrays in spec
	//shape: Array,
	//palette: Array,
	#[serde(default)]
	max_pixels_available: FilterRange<u32>,
}

pub fn list(
	boards: BoardDataMap,
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("boards")
		.and(warp::path::end())
		.and(warp::get())
		.and(warp::query())
		.and(warp::query())
		.and(authorized(users_db, Permission::BoardsList.into()))
		.then(move |pagination: PaginationOptions<BoardPageToken>, filter: BoardFilter, _, _| {
			let boards = Arc::clone(&boards);
			async move {
				let page = pagination.page.0;
				let limit = pagination
					.limit
					.unwrap_or(DEFAULT_PAGE_ITEM_LIMIT)
					.clamp(1, MAX_PAGE_ITEM_LIMIT);

				let boards = boards.read().await;
				let mut filtered_boards = vec![];
				for board in boards.values() {
					let lock = board.read().await;
					let info = &lock.as_ref().expect("Board went missing").info;

					let name_match = if let Some(ref name) = filter.name {
						&info.name == name
					} else {
						true
					};
					let create_match = filter.created_at
						.contains(info.created_at);
					let pixels_available_match = filter.max_pixels_available
						.contains(info.max_pixels_available);

					if name_match && create_match && pixels_available_match {
						filtered_boards.push(board);
					}
				}
				let mut pages = filtered_boards.chunks(limit)
					.skip(page.saturating_sub(2));
				
				fn page_uri(
					page: usize,
					limit: usize,
					filter: &BoardFilter,
				) -> Uri {
					let mut uri = format!(
						"/boards?page={}&limit={}",
						page, limit
					);

					if let Some(ref name) = filter.name {
						if let Some(name) = byte_serialize(name.as_bytes()).next() {
							uri.push_str(&format!("&name={}", name));
						}
					}
					
					if !filter.created_at.is_open() {
						uri.push_str(&format!("&created_at={}", filter.created_at));
					}
					
					if !filter.max_pixels_available.is_open() {
						uri.push_str(&format!("&max_pixels_available={}", filter.max_pixels_available));
					}

					uri.parse().unwrap()
				}

				let previous = page.checked_sub(1).and_then(|page| {
					pages.next().map(|_| page_uri(page, limit, &filter))
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
					.map(|b| Reference::new(Uri::from(b.deref()), b.deref()))
					.collect();

				let next = page.checked_add(1).and_then(|page| {
					pages.next().map(|_| page_uri(page, limit, &filter))
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
					&std::collections::HashMap::new(),
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

#[derive(Debug, Deserialize)]
struct SocketOptions {
	subscribe: Option<EnumSet<BoardSubscription>>,
	// TODO: use #serde(default) for this and similar
	authenticate: Option<bool>,
}

pub fn events(
	boards: BoardDataMap,
	boards_db: Arc<BoardsDatabase>,
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("boards")
		.and(board::path::read(&boards))
		.and(warp::path("events"))
		.and(warp::path::end())
		.and(warp::ws())
		.and(serde_qs::warp::query(Default::default()))
		.and(database::connection(boards_db))
		.map(move |board: PassableBoard, ws: Ws, options: SocketOptions, connection: BoardsConnection| {
			options.subscribe
				.ok_or(StatusCode::UNPROCESSABLE_ENTITY)
				.and_then(|subscriptions| {

					if subscriptions.is_empty() {
						return Err(StatusCode::UNPROCESSABLE_ENTITY);
					}

					let anonymous = !options.authenticate.unwrap_or(false);

					if anonymous {
						let permissions = Permission::defaults();
						let has_permissions = subscriptions.iter()
							.map(Permission::from)
							.all(|p| permissions.contains(p));
					
						if !has_permissions {
							return Err(StatusCode::FORBIDDEN);
						}
					}
				
					let users_db = Arc::clone(&users_db);
					let init_board = Arc::downgrade(&*board);
					let shutdown_board = Arc::downgrade(&*board);

					Ok(ws.on_upgrade(move |websocket| async move {
						let connect_result = Socket::connect(
							websocket,
							subscriptions,
							users_db,
							anonymous,
						).await;

						let socket = match connect_result {
							Ok(socket) => socket,
							Err(err) => return,
						};
						
						socket.init(|socket| async move {
							// add socket to board
							if let Some(board) = init_board.upgrade() {
								let mut board = board.write().await;
								let board = match *board {
									Some(ref mut board) => board,
									None => {
										let reason = Some(CloseReason::ServerClosing);
										socket.close(reason);
										return;
									},
								};
								
								let insert_result = board.insert_socket(
									&socket,
									&connection,
								).await;

								if let Err(err) = insert_result {
									let reason = Some(CloseReason::ServerError);
									socket.close(reason);
								}
							}
						}).await.shutdown(|socket| async move {
							// remove socket from board
							if let Some(board) = shutdown_board.upgrade() {
								let mut board = board.write().await;
								if let Some(ref mut board) = *board {
									board.remove_socket(&socket).await;
								}
							}
						}).await;
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
