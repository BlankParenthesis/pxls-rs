use super::*;

use web::{Path, Query, Data, Payload, Json};
use actix_web_actors::ws;
use std::collections::HashSet;
use std::convert::TryFrom;

use crate::socket::socket::{BoardSocket, Extension, SocketOptions};
use crate::socket::server::{RequestUserCount, Place};
use crate::BoardData;
use crate::BoardDataMap;

macro_rules! board {
	( $boards:ident[$id:ident] ) => {
		$boards.read().unwrap().get(&$id)
	}
}

pub mod data;
pub mod pixels;
pub mod users;

guard!(BoardListAccess, BoardsList);
guard!(BoardGetAccess, BoardsGet);
guard!(BoardPostAccess, BoardsPost);
guard!(BoardPatchAccess, BoardsPatch);
guard!(BoardDeleteAccess, BoardsDelete);
guard!(SocketAccess, SocketCore);

#[get("/boards")]
pub async fn list(
	Query(options): Query<PaginationOptions<usize>>,
	boards: BoardDataMap,
	_access: BoardListAccess,
) -> HttpResponse {
	let page = options.page.unwrap_or(0);
	let limit = options.limit.unwrap_or(10).clamp(1, 100);

	let boards = boards.read().unwrap();
	let boards = boards.iter()
		.map(|(id, BoardData(board, _))| (id, board.read().unwrap()))
		.collect::<Vec<_>>();
	let board_infos = boards.iter()
		.map(|(_id, board)| Reference::from(&**board))
		.collect::<Vec<_>>();
	let mut chunks = board_infos.chunks(limit);
	
	fn page_uri(page: usize, limit: usize) -> String {
		format!("/boards?page={}&limit={}", page, limit)
	}

	// TODO: standardize this
	HttpResponse::Ok()
		.json(Page {
			previous: page.checked_sub(1).and_then(
				|page| chunks
					.nth(page)
					.map(|_| page_uri(page, limit)),
			),
			items: chunks
				.next()
				.unwrap_or_default(),
			next: page.checked_add(1).and_then(
				|page| chunks
					.next()
					.map(|_| page_uri(page, limit)),
			),
		})
}

#[post("/boards")]
pub async fn post(
	Json(data): Json<BoardInfoPost>,
	boards: BoardDataMap,
	database_pool: Data<Pool>,
	_access: BoardPostAccess,
) -> Result<HttpResponse, Error> {
	// FIXME: properly raise the error, don't just expect.
	let board = Board::create(data, &database_pool.get().unwrap())
		.expect("create");
	let id = board.id as usize;

	let mut boards = boards.write().unwrap();
	boards.insert(id, BoardData::new(board));

	let BoardData(board, _) = boards.get(&id).unwrap();
	let board = board.read().unwrap();

	Ok(HttpResponse::build(StatusCode::CREATED)
		.header("Location", http::Uri::from(&*board).to_string())
		.json(Reference::from(&*board)))
}

#[get("/boards/default{rest:(/.*$)?}")]
pub async fn get_default(
	Path(rest): Path<String>,
	boards: BoardDataMap,
	_access: BoardGetAccess,
) -> Option<HttpResponse>  {
	boards.read().unwrap().keys().last().map(|id| {
		HttpResponse::TemporaryRedirect()
			.header("Location", format!("/boards/{}{}", id, rest))
			.finish()
	})
}

#[get("/boards/{id}")]
pub async fn get(
	Path(id): Path<usize>,
	boards: BoardDataMap,
	_access: BoardGetAccess,
) -> Option<HttpResponse> {
	board!(boards[id]).map(|BoardData(board, _)| {
		HttpResponse::Ok()
			.json(&board.read().unwrap().info)
	})
}

#[patch("/boards/{id}")]
pub async fn patch(
	// TODO: require application/merge-patch+json type?
	Json(data): Json<BoardInfoPatch>,
	Path(id): Path<usize>,
	boards: BoardDataMap,
	database_pool: Data<Pool>,
	_access: BoardPatchAccess,
) -> Option<HttpResponse> {
	board!(boards[id]).map(|BoardData(board, _)| {
		board.write().unwrap().update_info(
			data, 
			&database_pool.get().unwrap(),
		).expect("update");

		HttpResponse::Ok().json(&board.read().unwrap().info)
	})
}

#[delete("/boards/{id}")]
pub async fn delete(
	Path(id): Path<usize>,
	boards: BoardDataMap,
	database_pool: Data<Pool>,
	_access: BoardDeleteAccess,
) -> Option<HttpResponse> {
	boards.write().unwrap().remove(&id).map(|BoardData(board, _)| {
		board.into_inner().unwrap()
			.delete(&database_pool.get().unwrap()).unwrap();

		HttpResponse::new(StatusCode::NO_CONTENT)
	})
}

#[get("/boards/{id}/socket")]
pub async fn socket(
	Path(id): Path<usize>, 
	// FIXME: with `?extensions=…` this will fail with 400 rather than 422 as
	// it would with no query string.
	options: QsQuery<SocketOptions>,
	request: HttpRequest,
	stream: Payload,
	boards: BoardDataMap,
	_access: SocketAccess,
) -> Option<Result<HttpResponse, Error>> {
	board!(boards[id]).map(|BoardData(_, server)| {
		if let Some(extensions) = &options.extensions {
			let extensions: Result<HashSet<Extension>, _> = extensions
				.clone()
				.into_iter()
				.map(Extension::try_from)
				.collect();

			if let Ok(extensions) = extensions {
				ws::start(BoardSocket {
					extensions,
					server: server.clone()
				}, &request, stream)
			} else {
				Err(error::ErrorUnprocessableEntity(
					"Requested extensions not supported"
				))
			}
		} else {
			Err(error::ErrorUnprocessableEntity(
				"No extensions specified"
			))
		}
	})
}
