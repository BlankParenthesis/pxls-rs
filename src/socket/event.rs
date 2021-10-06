use actix::Message;
use serde::{Serialize, Serializer, ser::SerializeMap};

use crate::access::permissions::Permission;
use crate::objects::{VecShape, Palette};

#[derive(Serialize, Debug, Clone)]
pub struct Change<T> {
	pub position: u64,
	pub values: Vec<T>,
}

#[derive(Serialize, Debug, Clone)]
pub struct BoardInfo {
	#[serde(skip_serializing_if="Option::is_none")]
	pub name: Option<String>,
	#[serde(skip_serializing_if="Option::is_none")]
	pub shape: Option<VecShape>,
	#[serde(skip_serializing_if="Option::is_none")]
	pub palette: Option<Palette>,
}

#[derive(Serialize, Debug, Clone)]
pub struct BoardData {
	#[serde(skip_serializing_if="Option::is_none")]
	pub colors: Option<Vec<Change<u8>>>,
	#[serde(skip_serializing_if="Option::is_none")]
	pub timestamps: Option<Vec<Change<u32>>>,
	#[serde(skip_serializing_if="Option::is_none")]
	pub initial: Option<Vec<Change<u8>>>,
	#[serde(skip_serializing_if="Option::is_none")]
	pub mask: Option<Vec<Change<u8>>>,
}

#[derive(Message, Debug, Clone)]
#[rtype(result = "()")]
pub enum Event {
	BoardUpdate {
		info: Option<BoardInfo>,
		data: Option<BoardData>,
	},
	// TODO: send these
	PixelsAvailable {
		count: u32,
		next: Option<u64>,
	},
	PermissionsChanged {
		permissions: Vec<Permission>,
	},
}

impl Serialize for Event {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error> 
	where S: Serializer {
		match self {
			Event::BoardUpdate { info, data } => {
				let mut map = serializer.serialize_map(Some(2))?;
				map.serialize_entry("type", "board-update")?;
				if let Some(info) = info {
					map.serialize_entry("info", info)?;
				}
				if let Some(data) = data {
					map.serialize_entry("data", data)?;
				}
				map.end()
			},
			Event::PixelsAvailable { count, next } => {
				let mut map = serializer.serialize_map(Some(3))?;
				map.serialize_entry("type", "pixels-available")?;
				map.serialize_entry("count", count)?;
				map.serialize_entry("next", next)?;
				map.end()
			},
			Event::PermissionsChanged { permissions } => {
				let mut map = serializer.serialize_map(Some(2))?;
				map.serialize_entry("type", "permissions-changed")?;
				map.serialize_entry("permissions", permissions)?;
				map.end()
			},
		}
	}
}