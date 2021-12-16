use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

use crate::objects::{Extension, Palette, VecShape};

pub mod server {
	use super::*;

	#[derive(Serialize, Debug, Clone)]
	pub struct Change<T> {
		pub position: u64,
		pub values: Vec<T>,
	}

	#[skip_serializing_none]
	#[derive(Serialize, Debug, Clone)]
	pub struct BoardInfo {
		pub name: Option<String>,
		pub shape: Option<VecShape>,
		pub palette: Option<Palette>,
		pub max_stacked: Option<u32>,
	}

	#[skip_serializing_none]
	#[derive(Serialize, Debug, Clone)]
	pub struct BoardData {
		pub colors: Option<Vec<Change<u8>>>,
		pub timestamps: Option<Vec<Change<u32>>>,
		pub initial: Option<Vec<Change<u8>>>,
		pub mask: Option<Vec<Change<u8>>>,
	}

	#[skip_serializing_none]
	#[derive(Serialize, Debug, Clone)]
	#[serde(tag = "type")]
	#[serde(rename_all = "kebab-case")]
	pub enum Packet {
		BoardUpdate {
			info: Option<BoardInfo>,
			data: Option<BoardData>,
		},
		PixelsAvailable {
			count: u32,
			next: Option<u64>,
		},
		Ready,
	}

	impl From<&Packet> for Extension {
		fn from(event: &Packet) -> Self {
			match event {
				Packet::BoardUpdate { info, data } => Extension::Core,
				Packet::PixelsAvailable { count, next } => Extension::Core,
				Packet::Ready => Extension::Core,
			}
		}
	}
}

pub mod client {
	use super::*;

	#[derive(Deserialize, Debug, Clone)]
	#[serde(tag = "type")]
	#[serde(rename_all = "kebab-case")]
	pub enum Packet {
		Authenticate { token: Option<String> },
	}

	impl From<&Packet> for Extension {
		fn from(event: &Packet) -> Self {
			match event {
				Packet::Authenticate { token } => Extension::Authentication,
			}
		}
	}
}
