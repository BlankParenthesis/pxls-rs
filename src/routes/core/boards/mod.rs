use super::*;

use web::{Path, Query, Data, Payload, Json};
use actix_web_actors::ws;
use std::collections::HashSet;
use std::convert::TryFrom;
use std::sync::RwLock;

use crate::socket::socket::{Extension, SocketOptions};
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

// TODO: actix-web apparently deals very badly with diesel's blocking IO.
// Database operations should be wrapped in web::block and awaited.

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
		.map(|(id, board)| (id, board.read().unwrap()))
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
	let connection = &database_pool.get().unwrap();
	let board = Board::create(data, connection).unwrap();
	let id = board.id as usize;

	let mut boards = boards.write().unwrap();
	boards.insert(id, RwLock::new(board));

	let board = boards.get(&id).unwrap().read().unwrap();

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
	user: AuthedUser,
	database_pool: Data<Pool>,
	_access: BoardGetAccess,
) -> Option<HttpResponse> {
	board!(boards[id]).map(|board| {
		let board = board.read().unwrap();
		let connection = &database_pool.get().unwrap();
		let mut response = HttpResponse::Ok();
		
		if let AuthedUser::Authed(user) = user {
			let cooldown_info = board.user_cooldown_info(&user, connection)
				.unwrap();
	
			for (key, value) in cooldown_info.into_headers() {
				response.header(key, value);
			}
		}

		response.json(&board.info)
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
	board!(boards[id]).map(|board| {
		board.write().unwrap().update_info(
			data, 
			&database_pool.get().unwrap(),
		).unwrap();

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
	boards.write().unwrap().remove(&id).map(|board| {
		board.into_inner().unwrap()
			.delete(&database_pool.get().unwrap()).unwrap();

		HttpResponse::new(StatusCode::NO_CONTENT)
	})
}

#[get("/boards/{id}/socket")]
#[allow(clippy::too_many_arguments)] // humans don't call this function.
pub async fn socket(
	Path(id): Path<usize>, 
	options: QsQuery<SocketOptions>,
	request: HttpRequest,
	stream: Payload,
	user: AuthedUser,
	boards: BoardDataMap,
	database_pool: Data<Pool>,
	_access: SocketAccess,
) -> Option<Result<HttpResponse, Error>> {
	board!(boards[id]).map(|board| {
		let board = board.read().unwrap();
		if let Some(extensions) = &options.extensions {
			let connection = database_pool.get().unwrap();

			let extensions: Result<HashSet<Extension>, _> = extensions
				.clone()
				.into_iter()
				.map(Extension::try_from)
				.collect();

			// TODO: check client has permissions for all extensions.
			if let Ok(extensions) = extensions {
				let socket = board.new_socket(extensions, user.into(), &connection).unwrap();

				ws::start(socket, &request, stream)
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
