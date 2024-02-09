pub mod packet;

use core::hash::Hash;
use std::{
	sync::{Arc, Weak},
	time::{Duration, SystemTime},
};

use sea_orm::DatabaseConnection as Connection;
use async_trait::async_trait;
use enum_map::Enum;
use enumset::{EnumSet, EnumSetType};
use futures_util::{stream::SplitStream, FutureExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, RwLock};
use tokio_stream::wrappers::UnboundedReceiverStream;
use uuid::Uuid;
use warp::ws;

use crate::{
	permissions::Permission,
	openid::{ValidationError, self},
	board::Board,
	filter::header::authorization::Bearer,
};

// TODO: move this somewhere else
#[derive(Debug, EnumSetType, Enum, Deserialize, Serialize)]
#[enumset(serialize_repr = "list")]
#[serde(rename_all = "snake_case")]
pub enum Extension {
	Core,
	Authentication,
	BoardTimestamps,
	BoardMask,
	BoardInitial,
	BoardLifecycle,
}

impl Extension {
	pub fn socket_permission(&self) -> Permission {
		match self {
			Extension::Core => Permission::SocketCore,
			Extension::Authentication => Permission::SocketAuthentication,
			Extension::BoardTimestamps => Permission::SocketBoardsTimestamps,
			Extension::BoardMask => Permission::SocketBoardsMask,
			Extension::BoardInitial => Permission::SocketBoardsInitial,
			Extension::BoardLifecycle => Permission::SocketBoardLifecycle,
		}
	}
}

pub enum CloseReason {
	ServerClosing,
	InvalidPacket,
	AuthTimeout,
	MissingPermission,
	InvalidToken,
}

impl From<CloseReason> for u16 {
	fn from(reason: CloseReason) -> Self {
		match reason {
			CloseReason::ServerClosing => 1001,
			CloseReason::InvalidPacket => 1008,
			CloseReason::AuthTimeout => 4000,
			CloseReason::MissingPermission => 4001,
			CloseReason::InvalidToken => 4002,
		}
	}
}

enum Message {
	Close,
	Ping,
	Packet(packet::client::Packet),
	Invalid,
}

impl From<ws::Message> for Message {
	fn from(message: ws::Message) -> Message {
		if message.is_text() {
			let text = message.to_str().unwrap();
			match serde_json::from_str::<packet::client::Packet>(text) {
				Ok(packet) => Self::Packet(packet),
				Err(_) => Self::Invalid,
			}
		} else if message.is_ping() {
			Self::Ping
		} else if message.is_close() {
			Self::Close
		} else {
			Self::Invalid
		}
	}
}

#[async_trait]
trait MessageStream {
	async fn receive(&mut self) -> Option<Result<Message, ()>>;
}

#[async_trait]
impl MessageStream for SplitStream<ws::WebSocket> {
	async fn receive(&mut self) -> Option<Result<Message, ()>> {
		self.next().await
			.map(|o| o.map(Message::from).map_err(|_| ()))
	}
}

#[derive(Debug)]
enum AuthFailure {
	Timeout,
	Closed,
	InvalidMessage,
	Unauthorized,
	ValidationError(ValidationError),
}

pub struct UnauthedSocket {
	sender: mpsc::UnboundedSender<Result<ws::Message, warp::Error>>,
	extensions: EnumSet<Extension>,
}

impl UnauthedSocket {
	pub async fn connect(
		websocket: ws::WebSocket,
		extensions: EnumSet<Extension>,
		board: Weak<RwLock<Option<Board>>>,
		connection: Arc<Connection>,
	) {
		let (ws_sender, mut ws_receiver) = websocket.split();
		let (sender, sender_receiver) = mpsc::unbounded_channel();

		let sender_receiver = UnboundedReceiverStream::new(sender_receiver);

		tokio::task::spawn(
			sender_receiver
				.forward(ws_sender)
				.map(|result| {
					if let Err(e) = result {
						eprintln!("error sending websocket msg: {}", e);
					}
				}),
		);

		let socket = Self { sender, extensions };

		let timeout = tokio::time::sleep(Duration::from_secs(5));

		let auth_attempt = tokio::select! {
			_ = timeout => Err(AuthFailure::Timeout),
			socket = socket.authenticate_socket(&mut ws_receiver) => socket,
		};

		match auth_attempt {
			Ok(socket) => {
				let socket = Arc::new(socket);

				// add socket
				if let Some(board) = board.upgrade() {
					let mut board = board.write().await;
					if let Some(ref mut board) = *board {
						board.insert_socket(
							&socket,
							connection.as_ref(),
						).await.unwrap(); // TODO: bad unwrap? Handle by rejecting+closing connection.
					}
				}

				socket.handle_packets(&mut ws_receiver).await;

				// remove socket
				if let Some(board) = board.upgrade() {
					let mut board = board.write().await;
					if let Some(ref mut board) = *board {
						board.remove_socket(&socket).await;
					}
				}
			},
			Err(e) => {
				match e {
					AuthFailure::Timeout => todo!("4000"),
					AuthFailure::InvalidMessage => todo!("1003"),
					AuthFailure::Unauthorized => todo!("4001"),
					AuthFailure::ValidationError(_) => todo!("4002"),
					AuthFailure::Closed => (),
				}
			}
		}
	}

	fn authorize_user(
		&self,
		credentials: Option<Bearer>,
	) -> Result<Option<Bearer>, AuthFailure> {
		let permissions = match credentials {
			Some(ref bearer) => bearer.permissions(),
			None => Permission::defaults(),
		};

		let has_permission = self.extensions.iter()
			.map(|e| e.socket_permission())
			.all(|permission| permissions.contains(permission));

		if has_permission {
			Ok(credentials)
		} else {
			Err(AuthFailure::Unauthorized)
		}
	}

	async fn authenticate_user(
		token: String,
	) -> Result<Bearer, AuthFailure> {
		openid::validate_token(&token).await
			.map(Bearer::from)
			.map_err(AuthFailure::ValidationError)
	}

	async fn authenticate_socket(
		self,
		receiver: &mut SplitStream<ws::WebSocket>,
	) -> Result<AuthedSocket, AuthFailure> {
		if !self.extensions.contains(Extension::Authentication) {
			return Ok(AuthedSocket {
				uuid: Uuid::new_v4(),
				sender: self.sender,
				extensions: self.extensions,
				credentials: None,
			});
		}

		while let Some(Ok(msg)) = receiver.receive().await {
			use packet::client::Packet;
			match msg {
				Message::Packet(Packet::Authenticate { token }) => {
					let credentials = match token {
						Some(token) => {
							Some(UnauthedSocket::authenticate_user(token).await?)
						},
						None => None,
					};

					return self.authorize_user(credentials)
						.map(|credentials| AuthedSocket {
							uuid: Uuid::new_v4(),
							sender: self.sender,
							extensions: self.extensions,
							credentials: credentials.map(RwLock::new),
						});
				},
				Message::Packet(_) => return Err(AuthFailure::InvalidMessage),
				Message::Invalid => return Err(AuthFailure::InvalidMessage),
				Message::Close => (),
				Message::Ping => (),
			}
		}

		Err(AuthFailure::Closed)
	}
}

#[derive(Debug)]
pub struct AuthedSocket {
	uuid: Uuid,
	sender: mpsc::UnboundedSender<Result<ws::Message, warp::Error>>,
	pub extensions: EnumSet<Extension>,
	credentials: Option<RwLock<Bearer>>,
}

impl PartialEq for AuthedSocket {
	fn eq(
		&self,
		other: &Self,
	) -> bool {
		self.uuid == other.uuid
	}
}

impl Eq for AuthedSocket {}

impl Hash for AuthedSocket {
	fn hash<H: std::hash::Hasher>(
		&self,
		state: &mut H,
	) {
		self.uuid.hash(state);
	}
}

impl AuthedSocket {
	pub async fn user_id(&self) -> Option<String> {
		match self.credentials {
			Some(ref lock) => {
				Some(lock.read().await.id.clone())
			},
			None => None,
		}
	}

	pub async fn send(
		&self,
		message: &packet::server::Packet,
	) {
		let content = serde_json::to_string(message).unwrap();
		let message = ws::Message::text(content);

		if self.auth_valid().await {
			self.sender.send(Ok(message));
		} else {
			self.close(Some(CloseReason::InvalidToken));
		}
	}

	async fn auth_valid(&self) -> bool {
		match self.credentials {
			Some(ref lock) => lock.read().await.is_valid(),
			None => true,
		}
	}

	pub fn close(&self, reason: Option<CloseReason>) {
		let close = if let Some(reason) = reason {
			ws::Message::close_with(reason, "")
		} else {
			ws::Message::close()
		};

		self.sender.send(Ok(close));
	}

	// TODO: consider returning a result and closing based on that
	// side effects bad and all
	async fn reauthenticate(&self, token: Option<String>) {
		if !self.extensions.contains(Extension::Authentication) {
			self.close(Some(CloseReason::InvalidPacket));
			return;
		}

		match (token, self.credentials.as_ref()) {
			(Some(token), Some(credentials)) => {
				match UnauthedSocket::authenticate_user(token).await {
					Ok(user) => {
						let mut credentials = credentials.write().await;
						// NOTE: AuthedUser::eq tests only the subject
						// and not the expiry
						if *credentials.id == user.id {
							*credentials = user;
						} else {
							self.close(Some(CloseReason::InvalidToken));
						}
					},
					Err(_) => {
						self.close(Some(CloseReason::InvalidToken));
					},
				}
			},
			(None, None) => (),
			_ => self.close(Some(CloseReason::InvalidToken)),
		}
	} 

	async fn handle_packets(
		&self,
		receiver: &mut SplitStream<ws::WebSocket>,
	) {
		while let Some(Ok(msg)) = receiver.receive().await {
			use packet::client::Packet;
			match msg {
				Message::Packet(Packet::Authenticate { token }) => {
					self.reauthenticate(token).await;
				},
				Message::Invalid => {
					self.close(Some(CloseReason::InvalidPacket));
				},
				Message::Close => (),
				Message::Ping => (),
			}
		}
	}
}
