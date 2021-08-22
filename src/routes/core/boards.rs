use actix::Addr;
use actix_web::{http::header, web, get, HttpRequest, HttpResponse, Error, error::ErrorUnprocessableEntity};
use actix_web_actors::ws;
use std::collections::HashSet;
use std::convert::TryFrom;
use serde_qs::actix::QsQuery;

use crate::objects::paginated_list::{Page, PaginationOptions};
use crate::objects::board::Board;
use crate::socket::socket::{BoardSocket, Extension, SocketOptions};
use crate::socket::server::BoardServer;

guard!(BoardListAccess, BoardsList);
guard!(BoardGetAccess, BoardsGet);
guard!(BoardDataAccess, BoardsData);
guard!(SocketAccess, SocketCore);

#[get("/boards")]
pub async fn list(
	web::Query(options): web::Query<PaginationOptions>,
	board: web::Data<Board>,
	_access: BoardListAccess,
) -> HttpResponse {
	let page = options.page.unwrap_or(0);
	let limit = options.limit.unwrap_or(2).clamp(0, 10);

	let items = vec![&board.meta];
	let mut chunks = items.chunks(limit);
	
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
	web::Path(rest): web::Path<String>,
	_access: BoardGetAccess,
) -> Option<HttpResponse>  {
	Some(HttpResponse::TemporaryRedirect()
		.header("Location", format!("/boards/0{}", rest))
		.finish())
}

#[get("/boards/{id}")]
pub async fn get(
	web::Path(id): web::Path<u32>,
	board: web::Data<Board>,
	_access: BoardGetAccess,
) -> Option<HttpResponse> {
	if id == 0 {
		Some(HttpResponse::Ok().json(&board.meta))
	} else {
		None
	}
}

#[get("/boards/{id}/socket")]
pub async fn socket(
	web::Path(id): web::Path<u32>, 
	// FIXME: with `?extensions=…` this will fail with 400 rather than 422 as
	// it would with no query string.
	options: QsQuery<SocketOptions>,
	request: HttpRequest,
	stream: web::Payload,
	server: web::Data<Addr<BoardServer>>,
	_access: SocketAccess,
) -> Option<Result<HttpResponse, Error>> {
	if id == 0 {
		if let Some(extensions) = &options.extensions {
			let extensions: Result<HashSet<Extension>, _> = extensions
				.clone()
				.into_iter()
				.map(Extension::try_from)
				.collect();

			if let Ok(extensions) = extensions {
				Some(ws::start(BoardSocket {
					extensions,
					server
				}, &request, stream))
			} else {
				Some(Err(ErrorUnprocessableEntity(
					"Requested extensions not supported"
				)))
			}
		} else {
			Some(Err(ErrorUnprocessableEntity(
				"No extensions specified"
			)))
		}
	} else {
		None
	}
}

#[get("/boards/{id}/data/colors")]
pub async fn get_color_data(
	web::Path(id): web::Path<u32>,
	board: web::Data<Board>,
	_access: BoardDataAccess,
) -> Option<HttpResponse>  {
	if id == 0 {
		let disposition = header::ContentDisposition { 
			disposition: header::DispositionType::Attachment,
			parameters: vec![
				// TODO: maybe use the actual board name
				header::DispositionParam::Filename(String::from("board.dat")),
			],
		};

		Some(
			HttpResponse::Ok()
				.content_type("application/octet-stream")
				// TODO: if possible, work out how to use disposition itself for the name.
				.header("content-disposition", disposition)
				.body(board.data.lock().unwrap().colors.clone())
		)
	} else {
		None
	}
}

#[get("/boards/{id}/data/timestamps")]
pub async fn get_timestamp_data(
	web::Path(id): web::Path<u32>,
	board: web::Data<Board>,
	_access: BoardDataAccess,
) -> Option<HttpResponse>  {
	if id == 0 {
		Some(
			HttpResponse::Ok()
				.content_type("application/octet-stream")
				.body(board.data.lock().unwrap().timestamps.clone())
		)
	} else {
		None
	}
}

#[get("/boards/{id}/data/mask")]
pub async fn get_mask_data(
	web::Path(id): web::Path<u32>,
	board: web::Data<Board>,
	_access: BoardDataAccess,
) -> Option<HttpResponse>  {
	if id == 0 {
		Some(
			HttpResponse::Ok()
				.content_type("application/octet-stream")
				.body(board.data.lock().unwrap().mask.clone())
		)
	} else {
		None
	}
}
