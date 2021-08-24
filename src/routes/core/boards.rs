use actix::Addr;
use actix_web::{http::header, web, get, HttpRequest, HttpResponse, Error, error::ErrorUnprocessableEntity};
use actix_web_actors::ws;
use std::collections::HashSet;
use std::convert::TryFrom;
use serde_qs::actix::QsQuery;

use crate::objects::paginated_list::{Page, PaginationOptions};
use crate::objects::board::Board;
use crate::socket::socket::{BoardSocket, Extension, SocketOptions};
use crate::socket::server::{BoardServer, RequestUserCount};

guard!(BoardListAccess, BoardsList);
guard!(BoardGetAccess, BoardsGet);
guard!(BoardDataAccess, BoardsData);
guard!(BoardUsersAccess, BoardsUsers);
guard!(SocketAccess, SocketCore);

#[get("/boards")]
pub async fn list(
	web::Query(options): web::Query<PaginationOptions>,
	boards: web::Data<Vec<Board>>,
	_access: BoardListAccess,
) -> HttpResponse {
	let page = options.page.unwrap_or(0);
	let limit = options.limit.unwrap_or(2).clamp(0, 10);

	let board_infos = boards.iter()
		.map(|board| &board.info)
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
	web::Path(rest): web::Path<String>,
	_access: BoardGetAccess,
) -> Option<HttpResponse>  {
	Some(HttpResponse::TemporaryRedirect()
		.header("Location", format!("/boards/0{}", rest))
		.finish())
}

#[get("/boards/{id}")]
pub async fn get(
	web::Path(id): web::Path<usize>,
	boards: web::Data<Vec<Board>>,
	_access: BoardGetAccess,
) -> Option<HttpResponse> {
	boards.get(id).map(|board|
		HttpResponse::Ok().json(&board.info)
	)
}

#[get("/boards/{id}/socket")]
pub async fn socket(
	web::Path(id): web::Path<usize>, 
	// FIXME: with `?extensions=…` this will fail with 400 rather than 422 as
	// it would with no query string.
	options: QsQuery<SocketOptions>,
	request: HttpRequest,
	stream: web::Payload,
	server: web::Data<Addr<BoardServer>>,
	boards: web::Data<Vec<Board>>,
	_access: SocketAccess,
) -> Option<Result<HttpResponse, Error>> {
	boards.get(id).map(|board| {
		if let Some(extensions) = &options.extensions {
			let extensions: Result<HashSet<Extension>, _> = extensions
				.clone()
				.into_iter()
				.map(Extension::try_from)
				.collect();

			if let Ok(extensions) = extensions {
				ws::start(BoardSocket {
					extensions,
					server
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
	web::Path(id): web::Path<usize>,
	boards: web::Data<Vec<Board>>,
	_access: BoardDataAccess,
) -> Option<HttpResponse>  {
	boards.get(id).map(|board| {
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
	web::Path(id): web::Path<usize>,
	boards: web::Data<Vec<Board>>,
	_access: BoardDataAccess,
) -> Option<HttpResponse>  {
	boards.get(id).map(|board| {
		HttpResponse::Ok()
			.content_type("application/octet-stream")
			.body(board.data.read().unwrap().timestamps.clone())
	})
}

#[get("/boards/{id}/data/mask")]
pub async fn get_mask_data(
	web::Path(id): web::Path<usize>,
	boards: web::Data<Vec<Board>>,
	_access: BoardDataAccess,
) -> Option<HttpResponse>  {
	boards.get(id).map(|board| {
		HttpResponse::Ok()
			.content_type("application/octet-stream")
			.body(board.data.read().unwrap().mask.clone())
	})
}

#[get("/boards/{id}/data/initial")]
pub async fn get_initial_data(
	web::Path(id): web::Path<usize>,
	boards: web::Data<Vec<Board>>,
	_access: BoardDataAccess,
) -> Option<HttpResponse>  {
	boards.get(id).map(|board| {
		HttpResponse::Ok()
			.content_type("application/octet-stream")
			.body(board.data.read().unwrap().initial.clone())
	})
}

#[get("/boards/{id}/users")]
pub async fn get_users(
	web::Path(id): web::Path<usize>,
	boards: web::Data<Vec<Board>>,
	board_server: web::Data<Addr<BoardServer>>,
	_access: BoardUsersAccess,
) -> Option<HttpResponse>  {
	if let Some(board) = boards.get(id) {
		let user_count = board_server.send(RequestUserCount {}).await.unwrap();
		
		Some(HttpResponse::Ok().json(user_count))
	} else {
		None
	}
}
