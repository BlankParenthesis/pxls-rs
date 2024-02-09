use std::fmt;

use serde::{
	de::{self, Deserializer, Visitor},
	Deserialize, Serialize,
};

#[derive(Serialize, Debug)]
pub struct Page<'t, T> {
	pub items: &'t [T],
	pub next: Option<String>,
	pub previous: Option<String>,
}

#[derive(Deserialize, Debug)]
pub struct PaginationOptions<T> {
	pub page: Option<T>,
	pub limit: Option<usize>,
}

pub struct PageToken {
	pub id: usize,
	pub timestamp: u32,
}

impl PageToken {
	pub fn start() -> Self {
		Self {
			id: 0,
			timestamp: 0,
		}
	}
}

impl Default for PageToken {
	fn default() -> Self {
		Self::start()
	}
}

impl<'de> Deserialize<'de> for PageToken {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where
		D: Deserializer<'de>,
	{
		struct PageVisitor;

		impl<'de> Visitor<'de> for PageVisitor {
			type Value = PageToken;

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
						Ok(PageToken {
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
