use actix::{Context, Actor, Message, Recipient, Handler};
use std::collections::HashSet;

use crate::socket::event::Event;

#[derive(Default, Debug)]
pub struct BoardServer {
	connections: HashSet<Recipient<Event>>,
}

impl Actor for BoardServer {
	type Context = Context<Self>;
}

#[derive(Message)]
#[rtype(result = "()")]
pub struct Connect {
	pub handler: Recipient<Event>,
}

#[derive(Message)]
#[rtype(result = "()")]
pub struct Disconnect {
	pub handler: Recipient<Event>,
}

impl Handler<Connect> for BoardServer {
	type Result = ();

	fn handle(
		&mut self, 
		msg: Connect,
		_: &mut Self::Context,
	) -> Self::Result {
		msg.handler.do_send(Event::PermissionsChanged {
			permissions: vec![],
		}).unwrap();
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
