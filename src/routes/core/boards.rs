use actix_web::{
	http::header, 
	web::{Path, Query, Data, Payload}, 
	get, 
	HttpRequest, 
	HttpResponse, 
	Error, 
	error::ErrorUnprocessableEntity
};
use actix_web_actors::ws;
use std::collections::{HashSet, HashMap};
use std::convert::TryFrom;
use serde_qs::actix::QsQuery;

use crate::BoardData;
use crate::objects::{Page, PaginationOptions, Reference};
use crate::socket::socket::{BoardSocket, Extension, SocketOptions};
use crate::socket::server::RequestUserCount;

guard!(BoardListAccess, BoardsList);
guard!(BoardGetAccess, BoardsGet);
guard!(BoardDataAccess, BoardsData);
guard!(BoardUsersAccess, BoardsUsers);
guard!(SocketAccess, SocketCore);

#[get("/boards")]
pub async fn list(
	Query(options): Query<PaginationOptions>,
	boards: Data<HashMap<usize, BoardData>>,
	_access: BoardListAccess,
) -> HttpResponse {
	let page = options.page.unwrap_or(0);
	let limit = options.limit.unwrap_or(2).clamp(0, 10);

	let board_infos = boards.iter()
		.map(|(id, BoardData(board, _))| Reference::from(board))
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

#[get("/boards/default{rest:(/.*$)?}")]
pub async fn get_default(
	Path(rest): Path<String>,
	boards: Data<HashMap<usize, BoardData>>,
	_access: BoardGetAccess,
) -> Option<HttpResponse>  {
	boards.keys().last().map(|id| {
		HttpResponse::TemporaryRedirect()
			.header("Location", format!("/boards/{}{}", id, rest))
			.finish()
	})
}

#[get("/boards/{id}")]
pub async fn get(
	Path(id): Path<usize>,
	boards: Data<HashMap<usize, BoardData>>,
	_access: BoardGetAccess,
) -> Option<HttpResponse> {
	boards.get(&id).map(|BoardData(board, _)| {
		HttpResponse::Ok().json(&board.info)
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
	boards: Data<HashMap<usize, BoardData>>,
	_access: SocketAccess,
) -> Option<Result<HttpResponse, Error>> {
	boards.get(&id).map(|BoardData(_, server)| {
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
	boards: Data<HashMap<usize, BoardData>>,
	_access: BoardDataAccess,
) -> Option<HttpResponse>  {
	boards.get(&id).map(|BoardData(board, _)| {
		let disposition = header::ContentDisposition { 
			disposition: header::DispositionType::Attachment,
			parameters: vec![
				// TODO: maybe use the actual board name
				header::DispositionParam::Filename(String::from("board.dat")),
			],
		};

		HttpResponse::Ok()
			.content_type("application/octet-stream")
			// TODO: if possible, work out how to use disposition itself for the name.
			.header("content-disposition", disposition)
			.body(board.data.read().unwrap().colors.clone())
	})
}

#[get("/boards/{id}/data/timestamps")]
pub async fn get_timestamp_data(
	Path(id): Path<usize>,
	boards: Data<HashMap<usize, BoardData>>,
	_access: BoardDataAccess,
) -> Option<HttpResponse>  {
	boards.get(&id).map(|BoardData(board, _)| {
		HttpResponse::Ok()
			.content_type("application/octet-stream")
			.body(board.data.read().unwrap().timestamps.clone())
	})
}

#[get("/boards/{id}/data/mask")]
pub async fn get_mask_data(
	Path(id): Path<usize>,
	boards: Data<HashMap<usize, BoardData>>,
	_access: BoardDataAccess,
) -> Option<HttpResponse>  {
	boards.get(&id).map(|BoardData(board, _)| {
		HttpResponse::Ok()
			.content_type("application/octet-stream")
			.body(board.data.read().unwrap().mask.clone())
	})
}

#[get("/boards/{id}/data/initial")]
pub async fn get_initial_data(
	Path(id): Path<usize>,
	boards: Data<HashMap<usize, BoardData>>,
	_access: BoardDataAccess,
) -> Option<HttpResponse>  {
	boards.get(&id).map(|BoardData(board, _)| {
		HttpResponse::Ok()
			.content_type("application/octet-stream")
			.body(board.data.read().unwrap().initial.clone())
	})
}

#[get("/boards/{id}/users")]
pub async fn get_users(
	Path(id): Path<usize>,
	boards: Data<HashMap<usize, BoardData>>,
	_access: BoardUsersAccess,
) -> Option<HttpResponse>  {
	if let Some(BoardData(_, server)) = boards.get(&id) {
		let user_count = server.send(RequestUserCount {}).await.unwrap();
		
		Some(HttpResponse::Ok().json(user_count))
	} else {
		None
	}
}
