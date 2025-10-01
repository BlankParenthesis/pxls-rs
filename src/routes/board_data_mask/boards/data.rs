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
use crate::filter::header::range::{self, Range};
use crate::filter::body::patch;
use crate::filter::resource::board;
use crate::BoardDataMap;
use crate::filter::resource::board::PassableBoard;
use crate::filter::body::patch::BinaryPatch;
use crate::permissions::Permission;
use crate::database::{Database, DbConn, SectorBuffer};

pub fn get_mask(
	boards: BoardDataMap,
	db: Arc<Database>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("boards")
		.and(accept_encoding::gzip_opt())
		.and(board::path::read(&boards))
		.and(warp::path("data"))
		.and(warp::path("mask"))
		.and(warp::path::end())
		.and(warp::get())
		.and(
			warp::any()
				.and(range::range())
				.or(range::default())
				.unify(),
		)
		.and(authorized(db, Permission::BoardsDataGet.into()))
		.then(|gzip: bool, board: PassableBoard, range: Range, _, connection: DbConn| async move {
			// TODO: content disposition
			let board = board.read().await;
			let board = board.as_ref()
				.expect("Board went missing when getting mask data");
	
			let exact = board.try_read_exact_sector(range.clone(), SectorBuffer::Mask, true).await;
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
				let mut mask_data = board.read(SectorBuffer::Mask, &connection).await;
	
				range.respond_with(&mut mask_data).await.into_response()
			}
		})
}

pub fn patch_mask(
	boards: BoardDataMap,
	db: Arc<Database>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("boards")
		.and(board::path::read(&boards))
		.and(warp::path("data"))
		.and(warp::path("mask"))
		.and(warp::path::end())
		.and(patch::bytes())
		.and(authorized(db, Permission::BoardsDataPatch.into()))
		.then(|board: PassableBoard, patch: BinaryPatch, _, connection: DbConn| async move {
			// TODO: content disposition
			let board = board.write().await;
			board.as_ref()
				.expect("Board went missing when patching mask data")
				.try_patch_mask(&patch, &connection).await
				.map(|_| StatusCode::NO_CONTENT)
		})
}
