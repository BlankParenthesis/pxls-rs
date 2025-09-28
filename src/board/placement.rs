use std::fmt;

use serde::{Serialize, Deserialize};
use serde::de::{self, Deserializer, Visitor};

use crate::database::{User, UserSpecifier};
use crate::filter::response::paginated_list::PageToken;
use crate::filter::response::reference::Reference;

#[derive(Debug, Clone, Copy)]
pub struct CachedPlacement {
	pub position: u64,
	pub modified: u32,
	pub user: UserSpecifier,
}

impl Default for CachedPlacement {
	fn default() -> Self { Self::null() }
}

impl CachedPlacement {
	pub fn null() -> Self {
		Self {
			position: 0,
			modified: 0,
			user: UserSpecifier::null(),
		}
	}
}

#[derive(Debug, Clone, Copy)]
pub struct LastPlacement {
	pub id: i64,
	pub modified: u32,
	pub color: u8,
	pub user: UserSpecifier,
}

#[derive(Debug, Serialize, Clone)]
pub struct Placement {
	pub position: u64,
	pub color: u8,
	pub modified: u32,
	pub user: Reference<User>,
}

pub struct PlacementPageToken {
	pub id: usize,
	pub timestamp: u32,
}

impl PageToken for PlacementPageToken {
	fn start() -> Self {
		Self { id: 0, timestamp: 0 }
	}
}

impl Default for PlacementPageToken {
	fn default() -> Self { Self::start() }
}

impl fmt::Display for PlacementPageToken {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{}_{}", self.timestamp, self.id)
	}
}

impl<'de> Deserialize<'de> for PlacementPageToken {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where
		D: Deserializer<'de>,
	{
		struct PageVisitor;

		impl<'de> Visitor<'de> for PageVisitor {
			type Value = PlacementPageToken;

			fn expecting(
				&self,
				formatter: &mut fmt::Formatter,
			) -> fmt::Result {
				formatter.write_str("a string of two integers, separated by an underscore")
			}

			fn visit_str<E>(
				self,
				value: &str,
			) -> Result<Self::Value, E>
			where
				E: de::Error,
			{
				value.split_once('_')
					.ok_or_else(|| E::custom("missing underscore"))
					.and_then(|(timestamp, id)| {
						Ok(PlacementPageToken {
							id: id
								.parse()
								.map_err(|_| E::custom("id invalid"))?,
							timestamp: timestamp
								.parse()
								.map_err(|_| E::custom("timestamp invalid"))?,
						})
					})
			}
		}

		deserializer.deserialize_str(PageVisitor)
	}
}
