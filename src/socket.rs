pub mod packet;

use core::hash::Hash;
use std::{
	sync::{Arc, Weak},
	time::Duration, fmt,
};

use async_trait::async_trait;
use enum_map::Enum;
use enumset::{EnumSet, EnumSetType};
use futures_util::{stream::SplitStream, FutureExt, StreamExt};
use serde::de::{Deserialize, Visitor};
use tokio::sync::{mpsc, RwLock};
use tokio_stream::wrappers::UnboundedReceiverStream;
use uuid::Uuid;
use warp::ws;

use crate::{
	permissions::Permission,
	openid::{ValidationError, self},
	board::Board,
	filter::header::authorization::Bearer,
	database::{BoardsConnection, UsersDatabase, UsersConnection, Database},
};

#[derive(Debug, EnumSetType, Enum)]
#[enumset(serialize_repr = "list")]
pub enum BoardSubscription {
	DataColors,
	DataTimestamps,
	DataMask,
	DataInitial,
	Info,
	Cooldown,
}

impl TryFrom<&str> for BoardSubscription {
	type Error = ();

	fn try_from(value: &str) -> Result<Self, Self::Error> {
		match value {
			"data.colors" => Ok(BoardSubscription::DataColors),
			"data.timestamps" => Ok(BoardSubscription::DataTimestamps),
			"data.mask" => Ok(BoardSubscription::DataMask),
			"data.initial" => Ok(BoardSubscription::DataInitial),
			"info" => Ok(BoardSubscription::Info),
			"cooldown" => Ok(BoardSubscription::Cooldown),
			_ => Err(()),
		}
	}
}

// TODO: this format is quite common for things — maybe check if there's a
// crate to serialize with dots as separators already or create such a derive
// macro yourself.
// Update: strum looks maybe helpful but only has the same sort of
// transformations as serde by default.
impl<'de> Deserialize<'de> for BoardSubscription {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where D: serde::Deserializer<'de> {
		struct V;
		impl<'de> Visitor<'de> for V {
			type Value = BoardSubscription;

			fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
				write!(f, "A valid subscription string")
			}

			fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
			where E: serde::de::Error, {
				BoardSubscription::try_from(v)
					.map_err(|()| {
						format!("Invalid permission string \"{}\"", v)
					})
					.map_err(E::custom)
			}
		}

		deserializer.deserialize_str(V)
	}
}

// NOTE: this is needed for the correct deserialization to be set on enumtype
impl serde::Serialize for BoardSubscription {
	fn serialize<S>(&self, _: S) -> Result<S::Ok, S::Error>
	where S: serde::Serializer {
		unimplemented!()
	}
}

impl BoardSubscription {
	pub fn permission(&self) -> Permission {
		match self {
			BoardSubscription::DataColors => Permission::BoardsEventsDataColors,
			BoardSubscription::DataTimestamps => Permission::BoardsEventsDataTimestamps,
			BoardSubscription::DataMask => Permission::BoardsEventsDataMask,
			BoardSubscription::DataInitial => Permission::BoardsEventsDataInitial,
			BoardSubscription::Info => Permission::BoardsEventsInfo,
			BoardSubscription::Cooldown => Permission::BoardsEventsCooldown,
		}
	}
}

pub enum CloseReason {
	ServerClosing,
	ServerError,
	InvalidPacket,
	AuthTimeout,
	MissingPermission,
	InvalidToken,
}

impl From<CloseReason> for u16 {
	fn from(reason: CloseReason) -> Self {
		match reason {
			CloseReason::ServerClosing => 1001,
			CloseReason::ServerError => 1011,
			CloseReason::InvalidPacket => 1008,
			CloseReason::AuthTimeout => 4000,
			CloseReason::MissingPermission => 4001,
			CloseReason::InvalidToken => 4002,
		}
	}
}

#[derive(Debug)]
enum AuthFailure {
	Timeout,
	InvalidMessage,
	Unauthorized,
	TokenMismatch,
	ValidationError(ValidationError),
	ServerError,
	Closed,
}

impl From<AuthFailure> for Option<CloseReason> {
	fn from(failure: AuthFailure) -> Self {
		match failure {
			AuthFailure::Timeout => Some(CloseReason::AuthTimeout),
			AuthFailure::InvalidMessage => Some(CloseReason::InvalidPacket),
			AuthFailure::Unauthorized => Some(CloseReason::MissingPermission),
			AuthFailure::TokenMismatch => Some(CloseReason::InvalidToken),
			AuthFailure::ValidationError(_) => Some(CloseReason::InvalidToken),
			AuthFailure::ServerError => Some(CloseReason::ServerError),
			AuthFailure::Closed => None,
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
struct Authenticated(Bearer);

impl Authenticated {
	async fn authenticate(token: String) -> Result<Self, AuthFailure> {
		openid::validate_token(&token).await
			.map(Bearer::from)
			.map(Self)
			.map_err(AuthFailure::ValidationError)
	}
}

#[derive(Debug)]
struct Authorized(Option<Authenticated>);

impl Authorized {
	async fn authorize(
		credentials: Option<Authenticated>,
		permissions: &[Permission],
		connection: &mut UsersConnection,
	) -> Result<Self, AuthFailure> {
		let user_permissions = match credentials {
			Some(Authenticated(ref bearer)) => {
				connection.user_permissions(&bearer.id).await
					.map_err(|_| AuthFailure::ServerError)?
			},
			None => Permission::defaults(),
		};

		if permissions.iter().copied().all(|p| user_permissions.contains(p)) {
			Ok(Self(credentials))
		} else {
			Err(AuthFailure::Unauthorized)
		}
	}

	fn user_id(&self) -> Option<String> {
		self.0.as_ref()
			.map(|Authenticated(bearer)| String::from(&bearer.id))
	}

	fn is_valid(&self) -> bool {
		self.0.as_ref()
			.map(|Authenticated(bearer)| bearer.is_valid())
			.unwrap_or(true)
	}

	fn is_anonymous(&self) -> bool {
		self.0.is_none()
	}
}

pub struct UnauthedSocket {
	sender: mpsc::UnboundedSender<Result<ws::Message, warp::Error>>,
	subscriptions: EnumSet<BoardSubscription>,
	users_db: Arc<UsersDatabase>,
}

impl UnauthedSocket {
	pub async fn connect(
		websocket: ws::WebSocket,
		subscriptions: EnumSet<BoardSubscription>,
		board: Weak<RwLock<Option<Board>>>,
		connection: BoardsConnection,
		users_db: Arc<UsersDatabase>,
		anonymous: bool,
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

		let socket = Self { sender, subscriptions, users_db };

		match socket.auth_socket(&mut ws_receiver, anonymous).await {
			Ok(socket) => {
				let socket = Arc::new(socket);

				// add socket
				if let Some(board) = board.upgrade() {
					let mut board = board.write().await;
					if let Some(ref mut board) = *board {
						let insert_result = board.insert_socket(
							&socket,
							&connection,
						).await;

						if let Err(err) = insert_result {
							let message = ws::Message::close_with(
								u16::from(CloseReason::ServerError),
								"",
							);
							
							let _ = socket.sender.send(Ok(message));
							return;
						}
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
			Err((socket, err)) => {
				let message = match Option::<CloseReason>::from(err) {
					Some(reason) => ws::Message::close_with(u16::from(reason), ""),
					None => ws::Message::close(),
				};

				let _ = socket.sender.send(Ok(message));
			}
		}
	}

	async fn authenticate_socket(
		&self,
		receiver: &mut SplitStream<ws::WebSocket>,
	) -> Result<Option<Authenticated>, AuthFailure> {
		let timeout = tokio::time::sleep(Duration::from_secs(5));
		tokio::pin!(timeout);

		loop {
			let next = tokio::select! {
				_ = &mut timeout => Err(AuthFailure::Timeout),
				msg = receiver.receive() => {
					match msg {
						Some(Ok(msg)) => Ok(msg),
						Some(Err(_)) => Err(AuthFailure::InvalidMessage),
						None => Err(AuthFailure::Closed),
					}
				},
			};

			use packet::client::Packet;
			match next {
				Ok(Message::Packet(Packet::Authenticate { token })) => {
					let credentials = match token {
						Some(token) => {
							Some(Authenticated::authenticate(token).await?)
						},
						None => None,
					};

					return Ok(credentials);
				},
				Ok(Message::Packet(_)) => return Err(AuthFailure::InvalidMessage),
				Ok(Message::Invalid) => return Err(AuthFailure::InvalidMessage),
				Ok(Message::Close) => (),
				Ok(Message::Ping) => (),
				Err(e) => return Err(e),
			}
		}
	}

	async fn auth_socket(
		self,
		receiver: &mut SplitStream<ws::WebSocket>,
		anonymous: bool,
	) -> Result<AuthedSocket, (Self, AuthFailure)> {
		let credentials = if anonymous {
			None
		} else {
			match self.authenticate_socket(receiver).await {
				Ok(credentials) => credentials,
				Err(err) => return Err((self, err)),
			}
		};

		let permissions = self.subscriptions.iter()
			.map(|e| e.permission())
			.collect::<Vec<_>>();
	
		let mut connection = match self.users_db.connection().await {
			Ok(connection) => connection,
			Err(err) => return Err((self, AuthFailure::ServerError)),
		};

		match Authorized::authorize(credentials, &permissions, &mut connection).await {
			Ok(credentials) => {
				Ok(AuthedSocket {
					uuid: Uuid::new_v4(),
					sender: self.sender,
					subscriptions: self.subscriptions,
					credentials: RwLock::new(credentials),
					users_db: self.users_db,
				})
			},
			Err(e) => Err((self, e)),
		}
		
	}
}

pub struct AuthedSocket {
	uuid: Uuid,
	sender: mpsc::UnboundedSender<Result<ws::Message, warp::Error>>,
	pub subscriptions: EnumSet<BoardSubscription>,
	credentials: RwLock<Authorized>,
	users_db: Arc<UsersDatabase>,
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
		self.credentials.read().await.user_id()
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
		self.credentials.read().await.is_valid()
	}

	pub fn close(&self, reason: Option<CloseReason>) {
		let close = if let Some(reason) = reason {
			ws::Message::close_with(reason, "")
		} else {
			ws::Message::close()
		};

		let _ = self.sender.send(Ok(close));
	}

	async fn reauthorize(
		&self,
		token: Option<String>,
	) -> Result<(), AuthFailure> {
		if self.credentials.read().await.is_anonymous() {
			return Err(AuthFailure::InvalidMessage);
		}

		let permissions = self.subscriptions.iter()
			.map(|e| e.permission())
			.collect::<Vec<_>>();

		let mut connection = match self.users_db.connection().await {
			Ok(connection) => connection,
			Err(err) => return Err(AuthFailure::ServerError),
		};

		let new_credentials = match token {
			Some(token) => Some(Authenticated::authenticate(token).await?),
			None => None,
		};

		let new_credentials = Authorized::authorize(
			new_credentials,
			&permissions,
			&mut connection,
		).await?;

		let mut credentials = self.credentials.write().await;

		let old_id = credentials.user_id();
		let new_id = new_credentials.user_id();
		
		if old_id == new_id {
			*credentials = new_credentials;
			Ok(())
		} else {
			Err(AuthFailure::TokenMismatch)
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
					if let Err(err) = self.reauthorize(token).await {
						self.close(err.into());
					}
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
