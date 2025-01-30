mod packet;

use core::hash::Hash;
use std::{
	sync::Arc,
	time::Duration,
};

use async_trait::async_trait;
use enumset::{EnumSet, EnumSetType};
use futures_util::{stream::SplitStream, FutureExt, StreamExt};
use tokio::sync::{mpsc, RwLock};
use tokio_stream::wrappers::UnboundedReceiverStream;
use uuid::Uuid;
use warp::ws;

use crate::{
	permissions::Permission,
	openid::{ValidationError, self},
	filter::header::authorization::Bearer,
	database::{UsersDatabase, UsersConnection, Database},
};

use packet::ClientPacket;
pub use packet::ServerPacket;

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

#[derive(Debug, Clone)]
pub enum AuthFailure {
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
	Pong,
	Packet(ClientPacket),
	Invalid,
}

impl From<ws::Message> for Message {
	fn from(message: ws::Message) -> Message {
		if message.is_text() {
			let text = message.to_str().unwrap();
			match serde_json::from_str::<ClientPacket>(text) {
				Ok(packet) => Self::Packet(packet),
				Err(_) => Self::Invalid,
			}
		} else if message.is_ping() {
			Self::Ping
		} else if message.is_pong() {
			Self::Pong
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

// TODO: maybe move to auth module
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
		let user = credentials.as_ref().map(|Authenticated(bearer)| bearer.id.clone());
		let user_permissions = connection.user_permissions(user).await
			.map_err(|_| AuthFailure::ServerError)?;

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

#[must_use]
pub struct SocketWrapperInit<S: EnumSetType> {
	socket: Arc<Socket<S>>,
	receiver: SplitStream<ws::WebSocket>,
}

impl<S: EnumSetType> SocketWrapperInit<S> where Permission: From<S> {
	pub async fn init<F, O>(self, f: F) -> SocketWrapperShutdown<S>
	where
		F: FnOnce(Arc<Socket<S>>) -> O,
		O: std::future::Future<Output = ()>,
	{
		self.socket.ready().await;
		f(self.socket.clone()).await;
		self.socket.run(self.receiver).await;

		SocketWrapperShutdown {
			socket: self.socket,
		}
	}
}

#[must_use]
pub struct SocketWrapperShutdown<S: EnumSetType> {
	socket: Arc<Socket<S>>,
}

impl<S: EnumSetType> SocketWrapperShutdown<S> {
	pub async fn shutdown<F, O>(self, f: F)
	where
		F: FnOnce(Arc<Socket<S>>) -> O,
		O: std::future::Future<Output = ()>,
	{
		f(self.socket).await;
	}
}

pub struct Socket<S: EnumSetType> {
	uuid: Uuid,
	sender: mpsc::UnboundedSender<Result<ws::Message, warp::Error>>,
	pub subscriptions: EnumSet<S>,
	credentials: RwLock<Authorized>,
	users_db: Arc<UsersDatabase>,
}

impl<S: EnumSetType> PartialEq for Socket<S> {
	fn eq(&self, other: &Self) -> bool {
		self.uuid == other.uuid
	}
}

impl<S: EnumSetType> Eq for Socket<S> {}

impl<S: EnumSetType> Hash for Socket<S> {
	fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
		self.uuid.hash(state);
	}
}

impl<S: EnumSetType> Socket<S> where Permission: From<S> {
	pub async fn connect(
		websocket: ws::WebSocket,
		subscriptions: EnumSet<S>,
		users_db: Arc<UsersDatabase>,
		anonymous: bool,
	) -> Result<SocketWrapperInit<S>, AuthFailure> {
		let (ws_sender, mut ws_receiver) = websocket.split();
		let (sender, sender_receiver) = mpsc::unbounded_channel();

		let sender_receiver = UnboundedReceiverStream::new(sender_receiver);

		tokio::task::spawn(sender_receiver.forward(ws_sender));

		let auth_result = Socket::auth(
			&mut ws_receiver,
			sender,
			subscriptions,
			users_db,
			anonymous,
		).await;

		match auth_result {
			Ok(socket) => {
				Ok(SocketWrapperInit {
					socket: Arc::new(socket),
					receiver: ws_receiver,
				})
			},
			Err((err, sender)) => {
				let message = match Option::<CloseReason>::from(err.clone()) {
					Some(reason) => ws::Message::close_with(u16::from(reason), ""),
					None => ws::Message::close(),
				};

				let _ = sender.send(Ok(message));
				Err(err)
			}
		}
	}

	async fn auth(
		receiver: &mut SplitStream<ws::WebSocket>,
		sender: mpsc::UnboundedSender<Result<ws::Message, warp::Error>>,
		subscriptions: EnumSet<S>,
		users_db: Arc<UsersDatabase>,
		anonymous: bool,
	) -> Result<Self, (AuthFailure, mpsc::UnboundedSender<Result<ws::Message, warp::Error>>)> {

		let credentials = if anonymous {
			None
		} else {
			match Socket::authenticate_socket(receiver).await {
				Ok(credentials) => credentials,
				Err(err) => return Err((err, sender)),
			}
		};

		let permissions = subscriptions.iter()
			.map(Permission::from)
			.collect::<Vec<_>>();
	
		let mut connection = match users_db.connection().await {
			Ok(connection) => connection,
			Err(err) => return Err((AuthFailure::ServerError, sender)),
		};

		let authorize_attempt = Authorized::authorize(
			credentials,
			&permissions,
			&mut connection
		).await;

		match authorize_attempt {
			Ok(credentials) => {
				Ok(Socket {
					uuid: Uuid::new_v4(),
					sender,
					subscriptions,
					credentials: RwLock::new(credentials),
					users_db,
				})
			},
			Err(err) => Err((err, sender)),
		}
	}

	// TODO: maybe move this to Authenticated
	async fn authenticate_socket(
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

			match next {
				Ok(Message::Packet(ClientPacket::Authenticate { token })) => {
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
				Ok(Message::Pong) => (),
				Err(e) => return Err(e),
			}
		}
	}

	pub async fn user_id(&self) -> Option<String> {
		self.credentials.read().await.user_id()
	}

	pub async fn send<P: ServerPacket>(
		&self,
		message: &P,
	) {
		let content = serde_json::to_string(message).unwrap();
		let message = ws::Message::text(content);

		if self.auth_valid().await {
			let _ = self.sender.send(Ok(message));
		} else {
			self.close(Some(CloseReason::InvalidToken));
		}
	}

	async fn ready(&self) {
		let message = ws::Message::text(r#"{"type":"ready"}"#);

		if self.auth_valid().await {
			let _ = self.sender.send(Ok(message));
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
			.map(Permission::from)
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

	async fn run(
		&self,
		mut receiver: SplitStream<ws::WebSocket>,
	) {
		while let Some(Ok(msg)) = receiver.receive().await {
			match msg {
				Message::Packet(ClientPacket::Authenticate { token }) => {
					if let Err(err) = self.reauthorize(token).await {
						self.close(err.into());
					}
				},
				Message::Packet(ClientPacket::Ping) => {
					if let Err(_) = self.sender.send(Ok(warp::ws::Message::ping([]))) {
						self.close(None);
					}
				}
				Message::Invalid => {
					self.close(Some(CloseReason::InvalidPacket));
				},
				Message::Close => (),
				Message::Pong => (),
				Message::Ping => (),
			}
		}
	}
}
