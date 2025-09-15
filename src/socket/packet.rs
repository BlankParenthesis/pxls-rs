use serde::{Deserialize, Serialize};
use warp::filters::ws;

#[derive(Deserialize, Debug, Clone)]
#[serde(tag = "type")]
#[serde(rename_all = "kebab-case")]
pub enum ClientPacket {
	Authenticate { token: Option<String> },
	Ping,
}

pub trait ServerPacket: Serialize {
	fn serialize_packet(&self) -> SerializedPacket {
		let text = serde_json::to_string(&self).unwrap();
		SerializedPacket(ws::Message::text(text))
	}
}

pub struct SerializedPacket(ws::Message);

pub trait SerializedServerPacket {
	fn message(&self) -> ws::Message;
}

impl SerializedServerPacket for SerializedPacket {
	fn message(&self) -> ws::Message {
		self.0.clone()
	}
}
