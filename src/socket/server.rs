use actix::prelude::*;
use std::collections::{HashSet, HashMap};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use std::convert::TryFrom;
use actix::clock::Instant;

use std::sync::Arc;
use enum_map::EnumMap;

use crate::socket::event::Event;
use crate::socket::socket::Extension;
use crate::objects::board::CooldownInfo;

use crate::socket::socket::BoardSocket;

struct UserSockets {
	sockets: HashSet<Addr<BoardSocket>>,
	cooldown_timer: SpawnHandle,
}

#[derive(Default)]
pub struct BoardServer {
	connections_by_extension: EnumMap<Extension, HashSet<Addr<BoardSocket>>>,
	connections_by_user: HashMap<String, UserSockets>,
}

impl BoardServer {
	async fn timer(
		address: Addr<BoardServer>,
		user_id: String,
		mut cooldown_info: CooldownInfo,
	) {
		let mut next = cooldown_info.next();
		while let Some(time) = next {
			let instant = Instant::now() +
				time.duration_since(SystemTime::now())
					.unwrap_or(Duration::ZERO);
			let count = cooldown_info.pixels_available;
			actix::clock::delay_until(instant).await;
			
			next = cooldown_info.next();

			let event = Event::PixelsAvailable {
				count: u32::try_from(count).unwrap(),
				next: next.map(|time| time.duration_since(UNIX_EPOCH).unwrap()
					.as_secs()),
			};

			let user_id = Some(user_id.clone());

			address.do_send(RunEvent {
				event,
				user_id
			});
		}
	}

	fn all_connections(&self) -> HashSet<Addr<BoardSocket>> {
		let mut connections = HashSet::new();

		for socket in self.connections_by_extension.values() {
			connections.extend(socket.iter().cloned());
		}

		connections
	}
}

impl Actor for BoardServer {
	type Context = Context<Self>;
}

#[derive(Message)]
#[rtype(result = "()")]
pub struct Connect {
	pub user_id: Option<String>,
	pub socket: Addr<BoardSocket>,
	pub extensions: HashSet<Extension>,
	// TODO, maybe an Option(user_id, cooldown_info) since they're linked
	pub cooldown_info: Option<CooldownInfo>,
}

#[derive(Message)]
#[rtype(result = "()")]
pub struct Disconnect {
	pub user_id: Option<String>,
	pub socket: Addr<BoardSocket>,
	pub extensions: HashSet<Extension>,
}

#[derive(Message)]
#[rtype(result = "()")]
pub struct Cooldown {
	pub user_id: String,
	pub cooldown_info: CooldownInfo,
}

#[derive(Message)]
#[rtype(result = "()")]
pub struct RunEvent {
	pub user_id: Option<String>,
	pub event: Event,
}

#[derive(Message)]
#[rtype(result = "()")]
pub struct Close {}

impl Handler<Connect> for BoardServer {
	type Result = ();

	fn handle(
		&mut self, 
		Connect { extensions, user_id, socket, cooldown_info }: Connect,
		ctx: &mut Self::Context,
	) -> Self::Result {
		for extension in extensions {
			self.connections_by_extension[extension].insert(socket.clone());
		}

		socket.do_send(Arc::new(Event::Ready));

		if let Some(id) = user_id {
			let handle = BoardServer::timer(
				ctx.address(),
				id.clone(),
				cooldown_info.expect("Missing user cooldown info")
			).into_actor(self);

			let socket_group = self.connections_by_user.entry(id)
				.or_insert_with(|| UserSockets {
					sockets: Default::default(),
					cooldown_timer: ctx.spawn(handle),
				});
			socket_group.sockets.insert(socket);
		}
	}
}

impl Handler<Disconnect> for BoardServer {
	type Result = ();

	fn handle(
		&mut self, 
		Disconnect { extensions, user_id, socket }: Disconnect,
		ctx: &mut Self::Context,
	) -> Self::Result {
		for extension in extensions {
			self.connections_by_extension[extension].remove(&socket);
		}

		if let Some(id) = user_id {
			if let Some(socket_group) = self.connections_by_user.get_mut(&id) {
				socket_group.sockets.remove(&socket);
				if socket_group.sockets.is_empty() {
					ctx.cancel_future(socket_group.cooldown_timer);
					self.connections_by_user.remove(&id);
				}
			};
		}
	}
}

impl Handler<Close> for BoardServer {
	type Result = ();

	fn handle(
		&mut self, 
		_: Close,
		ctx: &mut Self::Context,
	) -> Self::Result {
		for connection in self.all_connections() {
			connection.do_send(Close {});
		}

		ctx.stop();
	}
}

impl Handler<RunEvent> for BoardServer {
	type Result = ();

	fn handle(
		&mut self, 
		RunEvent{ event, user_id }: RunEvent,
		_: &mut Self::Context,
	) -> Self::Result {
		let event = Arc::new(event);
		let connections = &self.connections_by_extension[event.as_ref().into()];
		// TODO: this is getting a bit ugly, perhaps storing the metadata on the
		// connection would be better after all 🤔.
		if let Some(user_id) = user_id {
			if let Some(group) = self.connections_by_user.get(&user_id) {
				for connection in connections.iter() {
					if group.sockets.contains(connection) {
						connection.do_send(event.clone());
					}
				}
			}
		} else {
			for connection in connections.iter() {
				connection.do_send(event.clone());
			}
		}

		// FIXME: handle case where max_stacked changed
	}
}

#[derive(Message)]
#[rtype(result = "()")]
struct Stop {}

impl Handler<Cooldown> for BoardServer {
	type Result = ();

	fn handle(
		&mut self, 
		Cooldown { mut cooldown_info, user_id }: Cooldown,
		ctx: &mut Self::Context,
	) -> Self::Result {
		let future = BoardServer::timer(ctx.address(), user_id.clone(), cooldown_info.clone())
			.into_actor(self);

		if let Some(socket_group) = self.connections_by_user.get_mut(&user_id) {
			ctx.cancel_future(socket_group.cooldown_timer);

			socket_group.cooldown_timer = ctx.spawn(future);


			let event = Event::PixelsAvailable {
				count: u32::try_from(cooldown_info.pixels_available).unwrap(),
				next: cooldown_info.next().map(|time| time.duration_since(UNIX_EPOCH).unwrap()
					.as_secs()),
			};

			let user_id = Some(user_id.clone());

			self.handle(RunEvent {
				event,
				user_id
			}, ctx);
		}
	}
}
