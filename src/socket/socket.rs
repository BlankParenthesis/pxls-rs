use actix::prelude::*;
use actix::{StreamHandler, Actor, AsyncContext, Handler, Addr};
use actix_web_actors::ws;
use serde::Deserialize;
use std::collections::HashSet;
use std::convert::TryFrom;
use std::fmt;
use std::sync::Arc;

use crate::socket::server::{BoardServer, Connect, Disconnect};
use crate::socket::event::Event;

#[derive(PartialEq, Eq, Hash, Debug, Clone)]
pub enum Extension {
	Core,
}

#[derive(Default, Debug)]
pub struct InvalidExtensionError;
impl fmt::Display for InvalidExtensionError {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "InvalidExtensionError")
	}
}

impl TryFrom<String> for Extension {
	type Error = InvalidExtensionError;
	fn try_from(string: String) -> Result<Self, Self::Error> {
		match string.as_str() {
			"core" => Ok(Extension::Core),
			_ => Err(Self::Error::default()),
		}
	}
}

#[derive(Deserialize)]
pub struct SocketOptions {
	pub extensions: Option<HashSet<String>>,
}

pub struct BoardSocket {
	pub extensions: HashSet<Extension>,
	pub server: Arc<Addr<BoardServer>>,
}

impl Handler<Arc<Event>> for BoardSocket {
	type Result = ();

	fn handle(
		&mut self, 
		msg: Arc<Event>,
		ctx: &mut Self::Context,
	) -> Self::Result {
		ctx.text(serde_json::to_string(&msg).unwrap())
	}
}

impl Actor for BoardSocket {
	type Context = ws::WebsocketContext<Self>;

	fn started(&mut self, ctx: &mut Self::Context) {
		self.server
			.send(Connect { handler: ctx.address().recipient() })
			.into_actor(self)
			.then(|res, _act, ctx| {
				if res.is_err() {
					ctx.stop();
				}
				fut::ready(())
			})
			.wait(ctx)
	}

	fn stopping(&mut self, ctx: &mut Self::Context) -> Running {
		self.server
			.do_send(Disconnect { handler: ctx.address().recipient() });
		Running::Stop
	}
}

impl StreamHandler<Result<ws::Message, ws::ProtocolError>> for BoardSocket {	
	fn handle(
		&mut self,
		msg: Result<ws::Message, ws::ProtocolError>,
		ctx: &mut Self::Context,
	) {
		match msg {
			Ok(ws::Message::Ping(msg)) => ctx.pong(&msg),
			// We don't expect any data from the client
			Ok(ws::Message::Text(_)) => {
				ctx.close(None);
				ctx.stop();
			},
			Ok(ws::Message::Binary(_)) => {
				ctx.close(None);
				ctx.stop();
			},
            Ok(ws::Message::Close(reason)) => {
                ctx.close(reason);
                ctx.stop();
            },
			_ => (),
		}
	}
}