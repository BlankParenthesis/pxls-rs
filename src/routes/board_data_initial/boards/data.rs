use std::sync::Arc;

use reqwest::header;
use warp::{
	http::StatusCode,
	reject::Rejection,
	reply::Reply,
	Filter,
};

use crate::filter::header::accept_encoding;
use crate::filter::header::authorization::authorized;
use crate::database::{UsersDatabase, BoardsDatabase, BoardsConnection};
use crate::filter::header::range::{self, Range};
use crate::filter::body::patch;
use crate::filter::resource::{board, database};
use crate::board::SectorBuffer;
use crate::BoardDataMap;
use crate::filter::resource::board::PassableBoard;
use crate::filter::body::patch::BinaryPatch;
use crate::permissions::Permission;

pub fn get_initial(
	boards: BoardDataMap,
	boards_db: Arc<BoardsDatabase>,
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("boards")
		.and(accept_encoding::gzip_opt())
		.and(board::path::read(&boards))
		.and(warp::path("data"))
		.and(warp::path("initial"))
		.and(warp::path::end())
		.and(warp::get())
		.and(
			warp::any()
				.and(range::range())
				.or(range::default())
				.unify(),
		)
		.and(authorized(users_db, Permission::BoardsDataGet.into()))
		.and(database::connection(boards_db))
		.then(|gzip: bool, board: PassableBoard, range: Range, _, _, connection: BoardsConnection| async move {
			// TODO: content disposition
			let board = board.read().await;
			let board = board.as_ref()
				.expect("Board went missing when getting initial data");
	
			let exact = board.try_read_exact_sector(range.clone(), SectorBuffer::Initial, true).await;
			if let Some((buffered, range)) = exact {
				match buffered.as_ref() {
					Ok(Some(sector)) => {
						let data = sector.initial.as_ref().unwrap();
						let range = format!("bytes {}-{}/{}", range.start, range.end, data.raw.len());
						
						let response = warp::hyper::Response::builder()
							.status(StatusCode::PARTIAL_CONTENT)
							.header(header::CONTENT_TYPE, "application/octet-stream")
							.header(header::CONTENT_RANGE, range);
						
						if gzip {
							response.header(header::CONTENT_ENCODING, "gzip")
								.body(data.compressed.clone())
								.unwrap()
								.into_response()
						} else {
							response.body(data.raw.clone())
								.unwrap()
								.into_response()
						}
					},
					Ok(None) => StatusCode::UNPROCESSABLE_ENTITY.into_response(),
					Err(e) => StatusCode::from(e).into_response(),
				}
			} else {
				let mut initial_data = board.read(SectorBuffer::Initial, &connection).await;
	
				range.respond_with(&mut initial_data).await.into_response()
			}
		})
}

pub fn patch_initial(
	boards: BoardDataMap,
	boards_db: Arc<BoardsDatabase>,
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("boards")
		.and(board::path::read(&boards))
		.and(warp::path("data"))
		.and(warp::path("initial"))
		.and(warp::path::end())
		.and(patch::bytes())
		.and(authorized(users_db, Permission::BoardsDataPatch.into()))
		.and(database::connection(boards_db))
		.then(|board: PassableBoard, patch: BinaryPatch, _, _, connection: BoardsConnection| async move {
			// TODO: content disposition
			let board = board.write().await;
			board.as_ref()
				.expect("Board went missing when patching initial data")
				.try_patch_initial(&patch, &connection).await
				.map(|_| StatusCode::NO_CONTENT)
		})
}
