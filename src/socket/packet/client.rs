use serde::Deserialize;

#[derive(Deserialize, Debug, Clone)]
#[serde(tag = "type")]
#[serde(rename_all = "kebab-case")]
pub enum Packet {
	Authenticate { token: Option<String> },
}
