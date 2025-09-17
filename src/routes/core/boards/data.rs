use std::sync::Arc;

use reqwest::StatusCode;
use reqwest::header;
use warp::{Filter, Reply, Rejection};

use crate::BoardDataMap;
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
		.then(|board: PassableBoard, range: Range, _, _, connection: BoardsConnection| async move {
			// TODO: content disposition
			let board = board.read().await;
			let board = board.as_ref()
				.expect("Board went missing when getting color data");
			
			if let Some((buffered, range)) = board.try_read_exact_sector(range.clone(), false).await {
				match buffered.as_ref() {
					Ok(Some(sector)) => {
						let range = format!("bytes {}-{}/{}", range.start, range.end, sector.colors.len());

						warp::hyper::Response::builder()
							.status(StatusCode::PARTIAL_CONTENT)
							.header(header::CONTENT_TYPE, "application/octet-stream")
							.header(header::CONTENT_RANGE, range)
							.body(sector.colors.to_vec())
							.unwrap()
							.into_response()
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
