use actix::Message;
use serde::{Serialize, Serializer, ser::SerializeMap};

use crate::database::model::Placement;
use crate::access::permissions::Permission;

#[derive(Message, Debug)]
#[rtype(result = "()")]
pub enum Event {
	BoardUpdate {
		pixels: Vec<Placement>,
	},
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
			Event::BoardUpdate { pixels } => {
				let mut map = serializer.serialize_map(Some(2))?;
				map.serialize_entry("type", "board-update")?;
				map.serialize_entry("pixels", pixels)?;
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