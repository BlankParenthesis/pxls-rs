use serde::Deserialize;
use crate::socket::Extension;

#[derive(Deserialize, Debug, Clone)]
#[serde(tag = "type")]
#[serde(rename_all = "kebab-case")]
pub enum Packet {
	Authenticate { token: Option<String> },
}

impl From<&Packet> for Extension {
	fn from(event: &Packet) -> Self {
		match event {
			Packet::Authenticate { .. } => Extension::Authentication,
		}
	}
}