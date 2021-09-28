use actix::{Context, Actor, Message, Recipient, Handler};
use std::collections::HashSet;

use crate::socket::event::Event;
use crate::objects::UserCount;
use crate::database::model;

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

#[derive(Message)]
#[rtype(result = "()")]
pub struct Place {
	pub placement: model::Placement,
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

impl Handler<Place> for BoardServer {
	type Result = ();

	fn handle(
		&mut self, 
		msg: Place,
		_: &mut Self::Context,
	) -> Self::Result {
		for connection in self.connections.iter() {
			connection.do_send(Event::BoardUpdate {
				// TODO: remove this clone — this doesn't need to be duplicated for every connection.
				pixels: vec![msg.placement.clone()],
			}).unwrap();
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
