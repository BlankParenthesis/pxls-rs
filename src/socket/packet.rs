use serde::{Deserialize, Serialize};

#[derive(Deserialize, Debug, Clone)]
#[serde(tag = "type")]
#[serde(rename_all = "kebab-case")]
pub enum ClientPacket {
	Authenticate { token: Option<String> },
}

pub trait ServerPacket: Serialize {}