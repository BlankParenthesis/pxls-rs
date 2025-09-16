use std::{
	convert::TryFrom,
	fmt,
};

use serde::de::{Deserialize, Visitor};
use enum_map::Enum;
use enumset::EnumSetType;

use crate::permissions::Permission;

#[derive(Debug, EnumSetType, Enum)]
#[enumset(serialize_repr = "list")]
pub enum BoardSubscription {
	DataColors,
	DataTimestamps,
	DataMask,
	DataInitial,
	Info,
	Cooldown,
	Notices,
	Statistics,
}

impl TryFrom<&str> for BoardSubscription {
	type Error = ();

	fn try_from(value: &str) -> Result<Self, Self::Error> {
		match value {
			"data.colors" => Ok(BoardSubscription::DataColors),
			"data.timestamps" => Ok(BoardSubscription::DataTimestamps),
			"data.mask" => Ok(BoardSubscription::DataMask),
			"data.initial" => Ok(BoardSubscription::DataInitial),
			"info" => Ok(BoardSubscription::Info),
			"cooldown" => Ok(BoardSubscription::Cooldown),
			"notices" => Ok(BoardSubscription::Notices),
			"statistics" => Ok(BoardSubscription::Statistics),
			_ => Err(()),
		}
	}
}

// TODO: this format is quite common for things — maybe check if there's a
// crate to serialize with dots as separators already or create such a derive
// macro yourself.
// Update: strum looks maybe helpful but only has the same sort of
// transformations as serde by default.
impl<'de> Deserialize<'de> for BoardSubscription {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where D: serde::Deserializer<'de> {
		struct V;
		impl<'de> Visitor<'de> for V {
			type Value = BoardSubscription;

			fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
				write!(f, "A valid subscription string")
			}

			fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
			where E: serde::de::Error, {
				BoardSubscription::try_from(v)
					.map_err(|()| {
						format!("Invalid permission string \"{}\"", v)
					})
					.map_err(E::custom)
			}
		}

		deserializer.deserialize_str(V)
	}
}

// NOTE: this is needed for the correct deserialization to be set on enumtype
impl serde::Serialize for BoardSubscription {
	fn serialize<S>(&self, _: S) -> Result<S::Ok, S::Error>
	where S: serde::Serializer {
		unimplemented!()
	}
}

impl From<BoardSubscription> for Permission {
	fn from(subscription: BoardSubscription) -> Permission {
		match subscription {
			BoardSubscription::DataColors => Permission::BoardsEventsDataColors,
			BoardSubscription::DataTimestamps => Permission::BoardsEventsDataTimestamps,
			BoardSubscription::DataMask => Permission::BoardsEventsDataMask,
			BoardSubscription::DataInitial => Permission::BoardsEventsDataInitial,
			BoardSubscription::Info => Permission::BoardsEventsInfo,
			BoardSubscription::Cooldown => Permission::BoardsEventsCooldown,
			BoardSubscription::Notices => Permission::BoardsEventsNotices,
			BoardSubscription::Statistics => Permission::BoardsEventsStatistics,
		}
	}
}
