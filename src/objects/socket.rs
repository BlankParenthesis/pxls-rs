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
	access::permissions::Permission,
	authentication::{openid::ValidationError, self},
	objects::{packet, AuthedUser, Board},
};

#[derive(Debug, EnumSetType, Enum, Deserialize, Serialize)]
#[enumset(serialize_repr = "list")]
#[serde(rename_all = "lowercase")]
pub enum Extension {
	Core,
	Authentication,
}

impl From<Extension> for Permission {
	fn from(extension: Extension) -> Permission {
		match extension {
			Extension::Core => Self::SocketCore,
			Extension::Authentication => Self::SocketAuthentication,
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

		if let Ok(socket) = auth_attempt {
			let socket = Arc::new(socket);

			// add socket
			if let Some(board) = board.upgrade() {
				let mut board = board.write().await;
				if let Some(ref mut board) = *board {
					board.insert_socket(
						Arc::clone(&socket),
						connection.as_ref(),
					).await.unwrap(); // TODO: bad unwrap? Handle by rejecting+closing connection.
				}
			}

			socket.handle_packets(&mut ws_receiver).await;

			// remove socket
			if let Some(board) = board.upgrade() {
				let mut board = board.write().await;
				if let Some(ref mut board) = *board {
					board.remove_socket(Arc::clone(&socket)).await;
				}
			}
		}
	}

	fn authorize_user(
		&self,
		authed_user: AuthedUser,
	) -> Result<AuthedUser, AuthFailure> {
		let actual_user = authed_user.user().unwrap_or_default();

		let has_permission = self.extensions.iter()
			.map(Permission::from)
			.all(|permission| {
				actual_user.permissions.contains(&permission)
			});

		if has_permission {
			Ok(authed_user)
		} else {
			Err(AuthFailure::Unauthorized)
		}
	}

	async fn authenticate_user(
		token: Option<String>,
	) -> Result<AuthedUser, AuthFailure> {
		if let Some(token) = token {
			authentication::openid::validate_token(&token).await
				.map(AuthedUser::from)
				.map_err(AuthFailure::ValidationError)
		} else {
			Ok(AuthedUser::None)
		}
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
				user: RwLock::new(AuthedUser::None),
			});
		}

		while let Some(Ok(msg)) = receiver.receive().await {
			use packet::client::Packet;
			match msg {
				Message::Packet(Packet::Authenticate { token }) => {
					let user = UnauthedSocket::authenticate_user(token).await
						.and_then(|user| self.authorize_user(user));
					return user.map(|user| AuthedSocket {
						uuid: Uuid::new_v4(),
						sender: self.sender,
						extensions: self.extensions,
						user: RwLock::new(user),
					})
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
	pub user: RwLock<AuthedUser>,
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
	pub async fn send(
		&self,
		message: &packet::server::Packet,
	) {
		let content = serde_json::to_string(message).unwrap();
		let message = ws::Message::text(content);

		if self.auth_valid().await {
			self.sender.send(Ok(message));
		} else {
			self.close();
		}
	}

	async fn auth_valid(&self) -> bool {
		let user = self.user.read().await;
		match &*user {
			AuthedUser::Authed {
				user: _,
				valid_until,
			} => SystemTime::now() < *valid_until,
			AuthedUser::None => true,
		}
	}

	pub fn close(&self) {
		self.sender.send(Ok(ws::Message::close()));
	}

	async fn reauthenticate(&self, token: Option<String>) {
		if self.extensions.contains(Extension::Authentication) {
			match UnauthedSocket::authenticate_user(token).await {
				Ok(user) => {
					let mut current_user = self.user.write().await;
					// NOTE: AuthedUser::eq tests only the subject
					// and not the expiry
					if *current_user == user {
						*current_user = user;
					} else {
						self.close();
					}
				},
				Err(_) => {
					self.close();
				},
			}
		} else {
			self.close();
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
					self.close();
				},
				Message::Close => (),
				Message::Ping => (),
			}
		}
	}
}
