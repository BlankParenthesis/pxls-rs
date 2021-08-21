
use actix::prelude::*;
use actix::{StreamHandler, Actor};
use actix_web::{web, get, HttpRequest, HttpResponse, Error, FromRequest};
use actix_web_actors::ws;
use serde::{Serialize, Deserialize, Deserializer, de::Visitor};
use std::time::{SystemTime, UNIX_EPOCH};
use std::collections::HashSet;
use std::convert::{TryFrom, TryInto};
use std::fmt;
use serde_qs::actix::QsQuery;

use crate::routes::core::pagination::{Page, PaginationOptions};

guard!(BoardListAccess, BoardsList);
guard!(BoardGetAccess, BoardsGet);
guard!(SocketAccess, SocketCore);

#[derive(Serialize)]
pub struct Color {
	name: String,
	value: u32,
}

#[derive(Serialize)]
pub struct BoardInfo {
	name: String,
	created_at: u64,
	shape: [[u64; 2]; 1],
	palette: Vec<Color>,
}

lazy_static! {
	pub static ref BOARD_INFO: BoardInfo = BoardInfo {
		name: String::from("c0"),
		created_at: SystemTime::now()
			.duration_since(UNIX_EPOCH)
			.unwrap()
			.as_millis() as u64,
		shape: [[1000, 1000]],
		palette: vec![Color { name: String::from("red"), value: 0xff0000 }],
	};
}


#[get("/boards")]
pub async fn list(web::Query(options): web::Query<PaginationOptions>, _access: BoardListAccess) -> HttpResponse {
	let page = options.page.unwrap_or(0);
	let limit = options.limit.unwrap_or(2).clamp(0, 10);
	
	let items = vec![&*BOARD_INFO, &*BOARD_INFO, &*BOARD_INFO, &*BOARD_INFO];
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

#[get("/boards/default")]
pub async fn get_default(_access: BoardGetAccess) -> Option<HttpResponse>  {
	Some(HttpResponse::TemporaryRedirect()
		.header("Location", "/boards/0")
		.finish())
}

#[get("/boards/{id}")]
pub async fn get(web::Path(id): web::Path<u32>, _access: BoardGetAccess) -> Option<HttpResponse> {
	if id == 0 {
		Some(HttpResponse::Ok().json(&*BOARD_INFO))
	} else {
		None
	}
}

#[derive(PartialEq, Eq, Hash, Debug, Clone)]
enum Extension {
	Core,
}

#[derive(Debug)]
struct E;
impl fmt::Display for E {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "InvalidExtensionError")
    }
}

impl TryFrom<String> for Extension {
	type Error = E;
	fn try_from(string: String) -> Result<Self, E> {
		match string.to_lowercase().as_str() {
			"core" => Ok(Extension::Core),
			_ => Err(E {}),
		}
	}
}

struct ExtensionVisitor;

impl<'de> Visitor<'de> for ExtensionVisitor {
	type Value = Extension;

	fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
		write!(formatter, "a string mapping to an extension name")
	}

	fn visit_str<E>(self, string: &str) -> Result<Self::Value, E>
	where E: serde::de::Error {
		match string.to_lowercase().as_str() {
			"core" => Ok(Extension::Core),
			_ => Err(serde::de::Error::invalid_value(
				serde::de::Unexpected::Str(string),
				&self,
			)),
		}
	}
}

impl<'de> Deserialize<'de> for Extension  {
	fn deserialize<S>(deserializer: S) -> Result<Self, S::Error>

	where S: Deserializer<'de> {
		deserializer.deserialize_str(ExtensionVisitor {})
	}
}

#[derive(Deserialize)]
pub struct SocketOptions {
	extensions: Option<HashSet<String>>,
}

#[derive(Default)]
struct BoardSocket {
	extensions: HashSet<Extension>,
}

impl Actor for BoardSocket {
	type Context = ws::WebsocketContext<Self>;
}

#[derive(Message)]
#[rtype(result = "()")]
struct PongRequest;

impl Handler<PongRequest> for BoardSocket {
	type Result = ();

	fn handle(
		&mut self,
		msg: PongRequest,
		ctx: &mut Self::Context,
	) {
		ctx.pong(&[])
	}
}

impl StreamHandler<Result<ws::Message, ws::ProtocolError>> for BoardSocket {	
	fn handle(
		&mut self,
		msg: Result<ws::Message, ws::ProtocolError>,
		ctx: &mut Self::Context,
	) {
		match msg {
			Ok(ws::Message::Ping(msg)) => {
				let recipient = ctx.address().recipient();
				let future = async move {
					recipient.do_send(PongRequest {}).unwrap();
				};
				future.into_actor(self).spawn(ctx);
			},
			Ok(ws::Message::Text(text)) => ctx.text(text),
			Ok(ws::Message::Binary(bin)) => ctx.binary(bin),
			_ => (),
		}
	}
}

#[get("/boards/{id}/socket")]
pub async fn socket(
	web::Path(id): web::Path<u32>, 
	// FIXME: with `?extensions=…` this will fail with 400 rather than 422 as it would with no query string.
	options: QsQuery<SocketOptions>,
	_access: SocketAccess,
	request: HttpRequest,
	stream: web::Payload
) -> Option<Result<HttpResponse, Error>> {
	if id == 0 {
		if let Some(extensions) = &options.extensions {
			let extensions: Result<HashSet<Extension>, _> = extensions
				.clone()
				.into_iter()
				.map(Extension::try_from)
				.collect();

			if let Ok(extensions) = extensions {
				Some(ws::start(BoardSocket { extensions }, &request, stream))
			} else {
				Some(Err(actix_web::error::ErrorUnprocessableEntity("Requested extensions not supported")))
			}
		} else {
			Some(Err(actix_web::error::ErrorUnprocessableEntity("No extensions specified")))
		}
	} else {
		None
	}
}
