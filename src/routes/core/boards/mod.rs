use std::sync::Arc;

use fragile::Fragile;
use http::header;
use parking_lot::RwLock;
use warp::path::Tail;

use super::*;
use crate::{
	filters::resource::board::{PassableBoard, PendingDelete},
	objects::socket::Extension,
	BoardDataMap,
};

pub mod data;
pub mod pixels;
pub mod users;

pub fn list(boards: BoardDataMap) -> impl Filter<Extract = impl Reply, Error = Rejection> + Clone {
	warp::path("boards")
		.and(warp::path::end())
		.and(warp::get())
		.and(authorization::bearer().and_then(with_permission(Permission::BoardsList)))
		.and(warp::query())
		.map(move |_user, pagination: PaginationOptions<usize>| {
			let page = pagination.page.unwrap_or(0);
			let limit = pagination
				.limit
				.unwrap_or(10)
				.clamp(1, 100);

			let boards = Arc::clone(&boards);
			let boards = boards.read();
			let boards = boards
				.iter()
				.map(|(id, board)| (id, board.read()))
				.collect::<Vec<_>>();
			let board_infos = boards
				.iter()
				.map(|(_id, board)| Reference::from(board.as_ref().unwrap()))
				.collect::<Vec<_>>();
			let mut chunks = board_infos.chunks(limit);

			fn page_uri(
				page: usize,
				limit: usize,
			) -> String {
				format!("/boards?page={}&limit={}", page, limit)
			}

			// TODO: standardize generation of this
			let response = Page {
				previous: page.checked_sub(1).and_then(|page| {
					chunks
						.nth(page)
						.map(|_| page_uri(page, limit))
				}),
				items: chunks.next().unwrap_or_default(),
				next: page.checked_add(1).and_then(|page| {
					chunks
						.next()
						.map(|_| page_uri(page, limit))
				}),
			};

			json(&response).into_response()
		})
}

pub fn default() -> impl Filter<Extract = impl Reply, Error = Rejection> + Clone {
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
	database_pool: Arc<Pool>,
) -> impl Filter<Extract = impl Reply, Error = Rejection> + Clone {
	warp::path("boards")
		.and(board::path::read(&boards))
		.and(warp::path::end())
		.and(warp::get())
		.and(authorization::bearer().and_then(with_permission(Permission::BoardsGet)))
		.and(database::connection(database_pool))
		.map(|board: PassableBoard, user, mut connection| {
			let board = board.read();
			let board = board.as_ref().unwrap();
			let mut response = json(&board.info).into_response();

			if let AuthedUser::Authed { user, valid_until } = user {
				let cooldown_info = board
					.user_cooldown_info(&user, &mut connection)
					.unwrap();

				for (key, value) in cooldown_info.into_headers() {
					response = reply::with_header(response, key, value).into_response();
				}
			}

			response
		})
}

pub fn post(
	boards: BoardDataMap,
	database_pool: Arc<Pool>,
) -> impl Filter<Extract = impl Reply, Error = Rejection> + Clone {
	warp::path("boards")
		.and(warp::path::end())
		.and(warp::post())
		.and(warp::body::json())
		.and(authorization::bearer().and_then(with_permission(Permission::BoardsPost)))
		.and(database::connection(database_pool))
		.map(move |data: BoardInfoPost, _user, mut connection| {
			let board = Board::create(data, &mut connection).unwrap();
			let id = board.id as usize;

			let boards = Arc::clone(&boards);
			let mut boards = boards.write();
			boards.insert(id, Arc::new(RwLock::new(Some(board))));

			let board = boards.get(&id).unwrap().read();
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
		})
}

pub fn patch(
	boards: BoardDataMap,
	database_pool: Arc<Pool>,
) -> impl Filter<Extract = impl Reply, Error = Rejection> + Clone {
	warp::path("boards")
		.and(board::path::read(&boards))
		.and(warp::path::end())
		.and(warp::patch())
		// TODO: require application/merge-patch+json type?
		.and(warp::body::json())
		.and(authorization::bearer().and_then(with_permission(Permission::BoardsPatch)))
		.and(database::connection(database_pool))
		.map(|board: PassableBoard, patch: BoardInfoPatch, _user, mut connection| {
			let mut board = board.write();
			let board = board.as_mut().unwrap();

			board.update_info(patch, &mut connection).unwrap();

			let mut response = json(&Reference::from(&*board)).into_response();
			response = reply::with_status(response, StatusCode::CREATED).into_response();
			response = reply::with_header(response, header::LOCATION, http::Uri::from(&*board).to_string()).into_response();
			response
		})
}

pub fn delete(
	boards: BoardDataMap,
	database_pool: Arc<Pool>,
) -> impl Filter<Extract = impl Reply, Error = Rejection> + Clone {
	warp::path("boards")
		.and(board::path::prepare_delete(&boards))
		.and(warp::path::end())
		.and(warp::delete())
		.and(authorization::bearer().and_then(with_permission(Permission::BoardsDelete)))
		.and(database::connection(database_pool))
		.map(
			move |deletion: Fragile<PendingDelete>, _user: AuthedUser, mut connection| {
				let mut deletion = deletion.into_inner();
				let board = deletion.perform();
				let mut board = board.write();
				let board = board.take().unwrap();
				board.delete(&mut connection).unwrap();
				StatusCode::NO_CONTENT.into_response()
			},
		)
}

#[derive(serde::Deserialize)]
pub struct SocketOptions {
	pub extensions: Option<enumset::EnumSet<Extension>>,
}

pub fn socket(
	boards: BoardDataMap,
	database_pool: Arc<Pool>,
) -> impl Filter<Extract = impl Reply, Error = Rejection> + Clone {
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
