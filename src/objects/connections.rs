use std::{
	collections::{HashMap, HashSet, hash_map::Entry},
	convert::TryFrom,
	sync::{Arc, Weak},
	time::{Duration, SystemTime, UNIX_EPOCH},
};

use enum_map::EnumMap;
use tokio::{time::Instant, sync::RwLock};
use tokio_util::sync::CancellationToken;

use super::{packet, packet::server::BoardDataCombination};
use super::{CooldownInfo, AuthedSocket, AuthedUser, Extension};

#[derive(Debug)]
struct UserConnections {
	connections: HashSet<Arc<AuthedSocket>>,
	cooldown_timer: Option<CancellationToken>,
}

impl UserConnections {
	async fn new(
		socket: Arc<AuthedSocket>,
		cooldown_info: CooldownInfo,
	) -> Arc<RwLock<Self>> {
		// NOTE: AuthedSocket hashes as the uuid, which is never mutated
		#[allow(clippy::mutable_key_type)]
		let mut connections = HashSet::new();
		connections.insert(socket);

		let user_connections = Arc::new(RwLock::new(Self {
			connections,
			cooldown_timer: None,
		}));

		Self::set_cooldown_info(
			Arc::clone(&user_connections),
			cooldown_info,
		).await;

		user_connections
	}

	fn insert(
		&mut self,
		socket: Arc<AuthedSocket>,
	) {
		self.connections.insert(socket);
	}

	fn remove(
		&mut self,
		socket: Arc<AuthedSocket>,
	) {
		self.connections.remove(&socket);
	}

	fn is_empty(&self) -> bool {
		self.connections.is_empty()
	}

	fn cleanup(&mut self) {
		assert!(self.is_empty());
		if let Some(timer) = self.cooldown_timer.take() {
			timer.cancel();
		}
	}

	async fn set_cooldown_info(
		connections: Arc<RwLock<Self>>,
		cooldown_info: CooldownInfo,
	) {
		let weak = Arc::downgrade(&connections);
		let new_token = CancellationToken::new();

		let cloned_token = CancellationToken::clone(&new_token);

		let mut connections = connections.write().await;

		let old_timer = connections.cooldown_timer.replace(new_token);
		if let Some(cancellable) = old_timer {
			cancellable.cancel();
		}

		let packet = packet::server::Packet::PixelsAvailable {
			count: u32::try_from(cooldown_info.pixels_available).unwrap(),
			next: cooldown_info
				.cooldown()
				.map(|timestamp| {
					timestamp
						.duration_since(UNIX_EPOCH)
						.unwrap()
						.as_secs()
				}),
		};

		connections.send(&packet).await;

		tokio::task::spawn(async move {
			tokio::select! {
				_ = cloned_token.cancelled() => (),
				_ = Self::cooldown_timer(weak, cooldown_info) => (),
			}
		});
	}

	async fn send(
		&self,
		packet: &packet::server::Packet,
	) {
		let extension = Extension::from(packet);
		for connection in &self.connections {
			if connection.extensions.contains(extension) {
				connection.send(packet).await;
			}
		}
	}

	async fn cooldown_timer(
		connections: Weak<RwLock<Self>>,
		mut cooldown_info: CooldownInfo,
	) {
		let mut next = cooldown_info.next();
		while let Some(time) = next {
			let instant = Instant::now()
				+ time
					.duration_since(SystemTime::now())
					.unwrap_or(Duration::ZERO);
			let count = cooldown_info.pixels_available;
			tokio::time::sleep_until(instant).await;

			next = cooldown_info.next();

			let packet = packet::server::Packet::PixelsAvailable {
				count: u32::try_from(count).unwrap(),
				next: next.map(|time| {
					time.duration_since(UNIX_EPOCH)
						.unwrap()
						.as_secs()
				}),
			};

			match connections.upgrade() {
				Some(connections) => {
					let connections = connections.write().await;
					connections.send(&packet).await;
				},
				None => {
					return;
				},
			}
		}
	}
}

#[derive(Debug, Default)]
pub struct Connections {
	by_uid: HashMap<String, Arc<RwLock<UserConnections>>>,
	by_extension: EnumMap<Extension, HashSet<Arc<AuthedSocket>>>,
	by_boarddata: EnumMap<BoardDataCombination, HashSet<Arc<AuthedSocket>>>,
}

impl Connections {
	pub async fn insert(
		&mut self,
		socket: &Arc<AuthedSocket>,
		cooldown_info: Option<CooldownInfo>,
	) {
		let user = socket.user.read().await;
		if let AuthedUser::Authed { user, .. } = &*user {
			if let Some(ref id) = user.id {
				let entry = self.by_uid.entry(id.clone());
				let connections = match entry {
					Entry::Vacant(entry) => {
						let new_connections = UserConnections::new(
							Arc::clone(socket),
							// SAFETY: this is only None if autheduser is None
							cooldown_info.unwrap(),
						).await;
						entry.insert(Arc::clone(&new_connections));
						new_connections
					},
					Entry::Occupied(entry) => Arc::clone(entry.get()),
				};

				connections.write().await.insert(Arc::clone(socket));
			}
		}

		for extension in socket.extensions {
			self.by_extension[extension].insert(Arc::clone(socket));
		}

		let combination = BoardDataCombination::from(socket.extensions);
		self.by_boarddata[combination].insert(Arc::clone(socket));
	}

	pub async fn remove(
		&mut self,
		socket: &Arc<AuthedSocket>,
	) {
		let user = socket.user.read().await;
		if let AuthedUser::Authed { user, .. } = &*user {
			if let Some(ref id) = user.id {
				let connections = self.by_uid.get(id).unwrap();
				let mut connections = connections.write().await;

				connections.remove(Arc::clone(socket));
				if connections.is_empty() {
					connections.cleanup();
					drop(connections);
					self.by_uid.remove(id);
				}
			}
		}

		for extension in socket.extensions {
			self.by_extension[extension].remove(socket);
		}

		let combination = BoardDataCombination::from(socket.extensions);
		self.by_boarddata[combination].remove(socket);
	}

	pub async fn send(
		&self,
		packet: packet::server::Packet,
	) {
		let extension = Extension::from(&packet);
		for connection in self.by_extension[extension].iter() {
			connection.send(&packet).await;
		}
	}

	pub async fn send_boarddata(
		&self,
		data: packet::server::BoardDataBuilder,
	) {
		for (combination, data) in data.build_combinations() {
			let packet = packet::server::Packet::BoardUpdate {
				info: None,
				data: Some(data),
			};

			for connection in self.by_boarddata[combination].iter() {
				connection.send(&packet).await;
			}
		}
	}

	pub async fn send_to_user(
		&self,
		user_id: String,
		packet: packet::server::Packet,
	) {
		if let Some(connections) = self.by_uid.get(&user_id) {
			connections.read().await.send(&packet).await;
		}
	}

	pub async fn set_user_cooldown(
		&self,
		user_id: String,
		cooldown_info: CooldownInfo,
	) {
		if let Some(connections) = self.by_uid.get(&user_id) {
			UserConnections::set_cooldown_info(
				Arc::clone(connections),
				cooldown_info,
			).await;
		}
	}

	pub fn close(&mut self) {
		// TODO: maybe send a close reason

		for connections in self.by_extension.values() {
			for connection in connections {
				connection.close();
			}
		}
	}
}