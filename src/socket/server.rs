use actix::{Context, Actor, Message, Recipient, Handler};
use std::collections::HashSet;

use std::sync::Arc;

use crate::socket::event::Event;
use crate::objects::UserCount;

#[derive(Default, Debug)]
pub struct BoardServer {
	connections: HashSet<Recipient<Arc<Event>>>,
}

impl Actor for BoardServer {
	type Context = Context<Self>;
}

// TODO: move messages to separate module?
#[derive(Message)]
#[rtype(result = "()")]
pub struct Connect {
	pub handler: Recipient<Arc<Event>>,
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
		self.connections.insert(msg.handler);
	}
}

impl Handler<Disconnect> for BoardServer {
	type Result = ();

	fn handle(
		&mut self, 
		msg: Disconnect,
		_: &mut Self::Context,
	) -> Self::Result {
		self.connections.remove(&msg.handler);
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
		for connection in self.connections.iter() {
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
			active: self.connections.len(),
			idle: 0,
			idle_timeout: 5 * 60,
		}
	}
}
