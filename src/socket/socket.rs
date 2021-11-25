use actix::prelude::*;
use actix::{StreamHandler, Actor, AsyncContext, Handler, Addr};
use actix_web_actors::ws;
use jsonwebtoken::TokenData;
use serde::Deserialize;
use std::collections::HashSet;
use std::convert::TryFrom;
use std::fmt;
use std::sync::{Arc, RwLock};
use enum_map::Enum;

use crate::database::Connection;
use crate::objects::{Board, User};
use crate::socket::server::{BoardServer, Connect, Disconnect};
use crate::socket::event::Event;
use crate::authentication::openid::{Identity, validate_token};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::socket::server::Close;

#[derive(PartialEq, Eq, Hash, Debug, Clone, Enum, Copy)]
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

pub struct BoardSocketInitInfo {
	pub board: Arc<RwLock<Option<Board>>>,
	pub database_connection: Connection,
}

pub struct BoardSocket {
	pub extensions: HashSet<Extension>,
	pub server: Arc<Addr<BoardServer>>,
	user_id: Option<String>,
	token_expiry: Option<SystemTime>,
	init_info: Option<BoardSocketInitInfo>,
}

impl BoardSocket {
	pub fn new(
		extensions: HashSet<Extension>,
		server: Arc<Addr<BoardServer>>,
		init_info: BoardSocketInitInfo,
	) -> Self {
		Self {
			extensions,
			server,
			user_id: None,
			token_expiry: None,
			init_info: Some(init_info),
		}
	}

	pub fn authenticated(&self) -> bool {
		self.init_info.is_none()
	}

	pub fn expired(&self) -> bool {
		if let Some(expiry) = self.token_expiry {
			expiry < SystemTime::now()
		} else {
			// anonymous sessions don't expire
			false
		}
	}

	fn auth(
		&mut self,
		token_data: Option<TokenData<Identity>>,
		ctx: &mut <Self as Actor>::Context,
	) -> Result<(), ()>{ 
		if let Some(identity) = token_data {
			let expiry = Duration::from_secs(u64::try_from(identity.claims.exp).unwrap());
			
			self.token_expiry = Some(UNIX_EPOCH + expiry);
			
			if self.authenticated() {
				if self.user_id.as_deref() == Some(identity.claims.sub).as_deref() {
					Ok(())
				} else {
					// no changing the user
					Err(())
				}
			} else {
				self.first_auth(Some(identity.claims.sub), ctx);
				Ok(())
			}
		} else if !self.authenticated() {
			// auth as anonymous
			self.first_auth(None, ctx);
			Ok(())
		} else {
			// no fallback to un-authed and no need to auth anonymous again
			Err(())
		}
	}

	fn first_auth(
		&mut self,
		user_id: Option<String>,
		ctx: &mut <Self as Actor>::Context,
	) {
		let BoardSocketInitInfo { board, database_connection } =
			self.init_info.take().expect("Missing init data for socket (maybe already consumed)");
			self.user_id = user_id;

		let board = board.read().unwrap();
		match board.as_ref() {
			Some(board) => {
				let cooldown_info = self.user_id.as_ref()
					.map(|user_id| User::from_id(user_id.clone()))
					.map(|user| board.user_cooldown_info(
						&user,
						&database_connection,
					).expect("Database failure when fetching cooldown"));

				// TODO: check client has permissions for all extensions.

				self.server
					.send(Connect {
						socket: ctx.address(),
						user_id: self.user_id.clone(),
						extensions: self.extensions.clone(),
						cooldown_info,
					})
					.into_actor(self)
					.then(|res, _act, ctx| {
						if res.is_err() {
							ctx.stop();
						}
						fut::ready(())
					})
					.wait(ctx);
			},
			None => {
				// the board was deleted before this auth occurred
				ctx.close(None);
				ctx.stop();
			},
		}
	}
}

impl Handler<Close> for BoardSocket {
	type Result = ();

	fn handle(
		&mut self, 
		_: Close,
		ctx: &mut Self::Context,
	) -> Self::Result {
		// TODO: maybe tell the client something about why we're closing the connection.
		// i.e, the board is being deleted, etc
		ctx.close(None);
		ctx.stop();
	}
}

impl Handler<Arc<Event>> for BoardSocket {
	type Result = ();

	fn handle(
		&mut self, 
		msg: Arc<Event>,
		ctx: &mut Self::Context,
	) -> Self::Result {
		if self.expired() {
			// NOTE: Clients are expected to refresh tokens and give them to
			// us of their own accord. If they haven't done so, tough luck:
			ctx.close(None);
			ctx.stop();
		} else {
			ctx.text(serde_json::to_string(&msg).unwrap())
		}
	}
}

impl Actor for BoardSocket {
	type Context = ws::WebsocketContext<Self>;

	fn started(&mut self, ctx: &mut Self::Context) {
		// NOTE: 5 seconds may be a bit aggressive, maybe increase to 30
		let require_auth_by = Instant::now() + Duration::from_secs(5);

		async move {
			actix::clock::delay_until(require_auth_by.into()).await;
		}.into_actor(self).then(|_, socket, ctx| {
			if !socket.authenticated() {
				// ло раззрогт, ло елтяу
				ctx.close(None);
				ctx.stop();
			}
			
			fut::ready(())
		}).spawn(ctx);
	}

	fn stopping(&mut self, ctx: &mut Self::Context) -> Running {
		if self.authenticated() {
			self.server
				.do_send(Disconnect {
					socket: ctx.address(),
					user_id: self.user_id.clone(),
					extensions: self.extensions.clone(),
				});
		}
		Running::Stop
	}
}

#[derive(Deserialize)]
struct AuthMessage {
	r#type: String,
	token: Option<String>,
}

impl StreamHandler<Result<ws::Message, ws::ProtocolError>> for BoardSocket {	
	fn handle(
		&mut self,
		msg: Result<ws::Message, ws::ProtocolError>,
		ctx: &mut Self::Context,
	) {
		match msg {
			Ok(ws::Message::Ping(msg)) => ctx.pong(&msg),
			Ok(ws::Message::Text(msg)) => {
				if let Ok(AuthMessage { r#type, token }) = serde_json::from_str(&msg) {
					// there's only one acceptable value
					if r#type != "authenticate" {
						ctx.close(None);
						ctx.stop();
					} else if let Some(token) = token {
						// auth as user
						async move { validate_token(&token).await }
							.into_actor(self)
							.then(|result, socket, ctx| {
								match result {
									Ok(identity) => {
										if socket.auth(Some(identity), ctx).is_err() {
											ctx.close(None);
											ctx.stop();
										}
									},
									Err(_) => {
										ctx.close(None);
										ctx.stop();
									}
								}
								fut::ready(())
							})
							.wait(ctx);
					} else if self.auth(None, ctx).is_err() {
						ctx.close(None);
						ctx.stop();
					}
				} else {
					ctx.close(None);
					ctx.stop();
				}
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