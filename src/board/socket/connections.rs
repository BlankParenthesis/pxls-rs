use std::{
	collections::{HashMap, HashSet, hash_map::Entry},
	convert::TryFrom,
	sync::{Arc, Weak},
	time::{Duration, SystemTime, UNIX_EPOCH},
};

use enum_map::EnumMap;
use enumset::EnumSet;
use tokio::{time::Instant, sync::{RwLock, mpsc}};
use tokio_util::sync::CancellationToken;

use crate::board::cooldown::CooldownInfo;
use crate::socket::CloseReason;

use super::BoardSubscription;
use super::packet::{Packet, DataType, BoardUpdateBuilder};

pub type Socket = crate::socket::Socket<BoardSubscription>;

struct UserConnections {
	connections: HashSet<Arc<Socket>>,
	cooldown_timer: Option<CancellationToken>,
}

impl UserConnections {
	async fn new(
		socket: Arc<Socket>,
		cooldown_info: Option<CooldownInfo>,
	) -> Arc<RwLock<Self>> {
		// NOTE: Socket hashes as the uuid, which is never mutated
		#[allow(clippy::mutable_key_type)]
		let mut connections = HashSet::new();
		connections.insert(socket);

		let user_connections = Arc::new(RwLock::new(Self {
			connections,
			cooldown_timer: None,
		}));

		if let Some(cooldown_info) = cooldown_info {
			Self::set_cooldown_info(
				Arc::clone(&user_connections),
				cooldown_info,
			).await;
		}

		user_connections
	}

	fn insert(
		&mut self,
		socket: Arc<Socket>,
	) {
		self.connections.insert(socket);
	}

	fn remove(
		&mut self,
		socket: Arc<Socket>,
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

		let packet = Packet::PixelsAvailable {
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
		packet: &Packet,
	) {
		let subscription = BoardSubscription::from(packet);
		for connection in &self.connections {
			if connection.subscriptions.contains(subscription) {
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

			let packet = Packet::PixelsAvailable {
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

type SocketList = HashSet<Arc<Socket>>;

pub struct Connections {
	by_uid: HashMap<Option<String>, Arc<RwLock<UserConnections>>>,
	by_subscription: EnumMap<BoardSubscription, SocketList>,
	by_board_update: Arc<RwLock<HashMap<EnumSet<DataType>, SocketList>>>,
	update_sender: mpsc::Sender<BoardUpdateBuilder>,
	// TODO: thread_join handle
}

impl Default for Connections {
	fn default() -> Self {
		let (update_sender, update_receiver) = mpsc::channel(10000);

		let by_board_update = Arc::default();
		tokio::spawn(Self::thread(Arc::clone(&by_board_update), update_receiver));

		Self {
			by_uid: HashMap::new(),
			by_subscription: EnumMap::default(),
			by_board_update,
			update_sender,
		}
	}
}

impl Connections {
	async fn thread(
		by_board_update: Arc<RwLock<HashMap<EnumSet<DataType>, SocketList>>>,
		mut receiver: mpsc::Receiver<BoardUpdateBuilder>,
	) {
		let mut buffer = vec![];
		while receiver.recv_many(&mut buffer, 10000).await > 0 {
			let mut data = BoardUpdateBuilder::default();

			for changes in buffer.drain(..) {
				data.merge(changes);
			}

			let sockets = by_board_update.read().await;

			for (combination, packet) in data.build_combinations() {
				if let Some(sockets) = sockets.get(&combination) {
					for connection in sockets {
						connection.send(&packet).await;
					}
				}
			}
		}
	}

	pub async fn insert(
		&mut self,
		socket: &Arc<Socket>,
		cooldown_info: Option<CooldownInfo>,
	) {
		let id = socket.user_id().await;
		let entry = self.by_uid.entry(id.clone());
		let connections = match entry {
			Entry::Vacant(entry) => {
				let new_connections = UserConnections::new(
					Arc::clone(socket),
					cooldown_info,
				).await;
				entry.insert(Arc::clone(&new_connections));
				new_connections
			},
			Entry::Occupied(entry) => Arc::clone(entry.get()),
		};

		connections.write().await.insert(Arc::clone(socket));

		for subscription in socket.subscriptions {
			self.by_subscription[subscription].insert(Arc::clone(socket));
		}

		let combination = socket.subscriptions.iter()
			.filter_map(Option::<DataType>::from)
			.collect();
		
		let mut board = self.by_board_update.write().await;
		board.entry(combination)
			.or_default()
			.insert(Arc::clone(socket));
	}

	pub async fn remove(
		&mut self,
		socket: &Arc<Socket>,
	) {
		let id = socket.user_id().await;
		let connections = self.by_uid.get(&id).unwrap();
		let mut connections = connections.write().await;

		connections.remove(Arc::clone(socket));
		if connections.is_empty() {
			connections.cleanup();
			drop(connections);
			self.by_uid.remove(&id);
		}

		for subscription in socket.subscriptions {
			self.by_subscription[subscription].remove(socket);
		}

		let combination = socket.subscriptions.iter()
			.filter_map(Option::<DataType>::from)
			.collect();
		
		let mut board = self.by_board_update.write().await;
		board.entry(combination)
			.or_default()
			.remove(socket);
	}

	pub async fn send(
		&self,
		packet: Packet,
	) {
		let subscription = BoardSubscription::from(&packet);
		for connection in self.by_subscription[subscription].iter() {
			connection.send(&packet).await;
		}
	}

	pub async fn queue_board_change(&self, data: BoardUpdateBuilder) {
		self.update_sender.send(data).await.expect("place event thread died");
	}

	pub async fn set_user_cooldown(
		&self,
		user_id: &str,
		cooldown_info: CooldownInfo,
	) {
		if let Some(connections) = self.by_uid.get(&Some(user_id.to_owned())) {
			UserConnections::set_cooldown_info(
				Arc::clone(connections),
				cooldown_info,
			).await;
		}
	}

	pub fn close(&mut self) {
		for connections in self.by_subscription.values() {
			for connection in connections {
				connection.close(Some(CloseReason::ServerClosing));
			}
		}
	}
}