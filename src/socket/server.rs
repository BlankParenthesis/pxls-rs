use actix::{Context, Actor, Message, Recipient, Handler};
use std::collections::HashSet;

use std::sync::Arc;
use enum_map::EnumMap;

use crate::socket::event::Event;
use crate::socket::socket::Extension;
use crate::objects::UserCount;

#[derive(Default, Debug)]
pub struct BoardServer {
	// TODO: respect extensions specification
	connections_by_extension: EnumMap<Extension, HashSet<Recipient<Arc<Event>>>>,
	//connections_by_user_id: HashMap<Option<String>, HashSet<Recipient<Arc<Event>>>>,
}

impl Actor for BoardServer {
	type Context = Context<Self>;
}

#[derive(Message)]
#[rtype(result = "()")]
pub struct Connect {
	pub handler: Recipient<Arc<Event>>,
	pub extensions: HashSet<Extension>,
}

#[derive(Message)]
#[rtype(result = "()")]
pub struct Disconnect {
	pub handler: Recipient<Arc<Event>>,
}

#[derive(Message)]
#[rtype(result = "()")]
pub struct RunEvent {
	pub event: Event,
}

impl Handler<Connect> for BoardServer {
	type Result = ();

	fn handle(
		&mut self, 
		msg: Connect,
		_: &mut Self::Context,
	) -> Self::Result {
		for extension in msg.extensions {
			self.connections_by_extension[extension].insert(msg.handler.clone());
		}
	}
}

impl Handler<Disconnect> for BoardServer {
	type Result = ();

	fn handle(
		&mut self, 
		msg: Disconnect,
		_: &mut Self::Context,
	) -> Self::Result {
		for (_, connections) in self.connections_by_extension.iter_mut() {
			connections.remove(&msg.handler);
		}
	}
}

impl Handler<RunEvent> for BoardServer {
	type Result = ();

	fn handle(
		&mut self, 
		msg: RunEvent,
		_: &mut Self::Context,
	) -> Self::Result {
		let event = Arc::new(msg.event);
		let connections = &self.connections_by_extension[event.as_ref().into()];
		for connection in connections.iter() {
			connection.do_send(event.clone()).unwrap();
		}
	}
}

#[derive(Message)]
#[rtype(result = "UserCount")]
pub struct RequestUserCount;

impl Handler<RequestUserCount> for BoardServer {
	type Result = UserCount;

	fn handle(
		&mut self, 
		_: RequestUserCount,
		_: &mut Self::Context,
	) -> Self::Result {
		UserCount {
			active: self.connections_by_extension[Extension::Core].len(),
			idle: 0,
			idle_timeout: 5 * 60,
		}
	}
}
