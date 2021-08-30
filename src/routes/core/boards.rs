use actix_web::{
	http::header::{self, Header}, 
	web::{Path, Query, Data, Payload, Json, Bytes}, 
	get,
	post, 
	patch,
	delete,
	HttpRequest, 
	HttpResponse,
	Error, 
	error::ErrorUnprocessableEntity
};
use actix_web_actors::ws;
use std::collections::{HashSet, HashMap};
use std::convert::TryFrom;
use std::sync::RwLock;
use serde_qs::actix::QsQuery;
use http::StatusCode;

use crate::BoardData;
use crate::objects::{Page, PaginationOptions, Reference, BoardInfoPost, BoardInfoPatch, Board, RangeHeader, HttpRange, TryIndex};
use crate::socket::socket::{BoardSocket, Extension, SocketOptions};
use crate::socket::server::RequestUserCount;
use crate::database::queries::Pool;

guard!(BoardListAccess, BoardsList);
guard!(BoardGetAccess, BoardsGet);
guard!(BoardPostAccess, BoardsPost);
guard!(BoardPatchAccess, BoardsPatch);
guard!(BoardDeleteAccess, BoardsDelete);
guard!(BoardDataAccess, BoardsData);
guard!(BoardUsersAccess, BoardsUsers);
guard!(SocketAccess, SocketCore);

macro_rules! board {
	( $boards:ident[$id:ident] ) => {
		$boards.read().unwrap().get(&$id)
	}
}

type BoardDataMap = Data<RwLock<HashMap<usize, BoardData>>>;

#[get("/boards")]
pub async fn list(
	Query(options): Query<PaginationOptions>,
	boards: BoardDataMap,
	_access: BoardListAccess,
) -> HttpResponse {
	let page = options.page.unwrap_or(0);
	let limit = options.limit.unwrap_or(2).clamp(0, 10);

	let boards = boards.read().unwrap();
	let boards = boards.iter()
		.map(|(id, BoardData(board, _))| (id, board.read().unwrap()))
		.collect::<Vec<_>>();
	let board_infos = boards.iter()
		.map(|(id, board)| Reference::from(&**board))
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
				.unwrap_or(&[]),
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
	pool: Data<Pool>,
	_access: BoardPostAccess,
) -> Result<HttpResponse, Error> {
	// FIXME: properly raise the error, don't just expect.
	let board = Board::create(data, &mut pool.get().expect("pool")).expect("create");
	let id = board.id;

	let mut boards = boards.write().unwrap();
	boards.insert(id, BoardData::new(board));

	let BoardData(board, _) = boards.get(&id).unwrap();
	let board = board.read().unwrap();

	Ok(HttpResponse::build(StatusCode::CREATED)
		.header("Location", format!("/boards/{}", board.id))
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
	Json(data): Json<BoardInfoPatch>,
	Path(id): Path<usize>,
	boards: BoardDataMap,
	pool: Data<Pool>,
	_access: BoardPatchAccess,
) -> Option<HttpResponse> {
	board!(boards[id]).map(|BoardData(board, _)| {
		board.write().unwrap().update_info(
			data, 
			&mut pool.get().expect("pool"),
		).expect("update");

		HttpResponse::Ok().json(&board.read().unwrap().info)
	})
}

#[delete("/boards/{id}")]
pub async fn delete(
	Path(id): Path<usize>,
	boards: BoardDataMap,
	pool: Data<Pool>,
	_access: BoardDeleteAccess,
) -> Option<HttpResponse> {
	boards.write().unwrap().remove(&id).map(|BoardData(board, _)| {
		board.into_inner().unwrap().delete(&mut pool.get().expect("pool")).unwrap();

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
				Err(ErrorUnprocessableEntity(
					"Requested extensions not supported"
				))
			}
		} else {
			Err(ErrorUnprocessableEntity(
				"No extensions specified"
			))
		}
	})
}

#[get("/boards/{id}/data/colors")]
pub async fn get_color_data(
	Path(id): Path<usize>,
	boards: BoardDataMap,
	range: RangeHeader,
	_access: BoardDataAccess,
) -> Option<HttpResponse>  {
	board!(boards[id]).map(|BoardData(board, _)| {
		range.respond_with(&board.read().unwrap().data.colors)
	})
}

#[get("/boards/{id}/data/timestamps")]
pub async fn get_timestamp_data(
	Path(id): Path<usize>,
	boards: BoardDataMap,
	range: RangeHeader,
	_access: BoardDataAccess,
) -> Option<HttpResponse>  {
	board!(boards[id]).map(|BoardData(board, _)| {
		range.respond_with(&board.read().unwrap().data.timestamps)
	})
}

#[get("/boards/{id}/data/mask")]
pub async fn get_mask_data(
	Path(id): Path<usize>,
	boards: BoardDataMap,
	range: RangeHeader,
	_access: BoardDataAccess,
) -> Option<HttpResponse>  {
	board!(boards[id]).map(|BoardData(board, _)| {
		range.respond_with(&board.read().unwrap().data.mask)
	})
}

#[get("/boards/{id}/data/initial")]
pub async fn get_initial_data(
	Path(id): Path<usize>,
	boards: BoardDataMap,
	range: RangeHeader,
	_access: BoardDataAccess,
) -> Option<HttpResponse>  {
	board!(boards[id]).map(|BoardData(board, _)| {
		range.respond_with(&board.read().unwrap().data.initial)
	})
}

#[get("/boards/{id}/users")]
pub async fn get_users(
	Path(id): Path<usize>,
	boards: BoardDataMap,
	_access: BoardUsersAccess,
) -> Option<HttpResponse>  {
	if let Some(BoardData(_, server)) = board!(boards[id]) {
		let user_count = server.send(RequestUserCount {}).await.unwrap();
		
		Some(HttpResponse::Ok().json(user_count))
	} else {
		None
	}
}
