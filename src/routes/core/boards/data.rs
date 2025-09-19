use std::sync::Arc;

use reqwest::StatusCode;
use reqwest::header;
use warp::{Filter, Reply, Rejection};

use crate::BoardDataMap;
use crate::filter::header::accept_encoding;
use crate::filter::header::authorization::authorized;
use crate::filter::header::range::{self, Range};
use crate::filter::resource::board::{self, PassableBoard};
use crate::filter::resource::database;
use crate::permissions::Permission;
use crate::board::SectorBuffer;
use crate::database::{BoardsDatabase, BoardsConnection, UsersDatabase};

pub fn get_colors(
	boards: BoardDataMap,
	boards_db: Arc<BoardsDatabase>,
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("boards")
		.and(accept_encoding::gzip_opt())
		.and(board::path::read(&boards))
		.and(warp::path("data"))
		.and(warp::path("colors"))
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
				.expect("Board went missing when getting color data");
				
			let exact = board.try_read_exact_sector(range.clone(), SectorBuffer::Colors, false).await;
			if let Some((buffered, range)) = exact {
				match buffered.as_ref() {
					Ok(Some(data)) => {
						let range = format!("bytes {}-{}/{}", range.start, range.end, data.raw.len());
						
						let response = warp::hyper::Response::builder()
							.status(StatusCode::PARTIAL_CONTENT)
							.header(header::CONTENT_TYPE, "application/octet-stream")
							.header(header::CONTENT_RANGE, range);
						
						if gzip {		
							response.header(header::CONTENT_ENCODING, "gzip")
								.body(data.compressed.clone()).unwrap()
								.into_response()
						} else {
							response.body(data.raw.clone()).unwrap()
								.into_response()
						}
					},
					Ok(None) => StatusCode::UNPROCESSABLE_ENTITY.into_response(),
					Err(e) => StatusCode::from(e).into_response(),
				}
			} else {
				let mut colors_data = board.read(SectorBuffer::Colors, &connection).await;
	
				range.respond_with(&mut colors_data).await.into_response()
			}
		})
}
