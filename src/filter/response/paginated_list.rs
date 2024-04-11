use serde::{Deserialize, Serialize, Serializer};
use warp::http::Uri;

use super::reference::Reference;

fn optional_uri<S>(uri: &Option<Uri>, ser: S) -> Result<S::Ok, S::Error>
where S: Serializer {
	if let Some(uri) = uri {
		http_serde::uri::serialize(uri, ser)
	} else {
		ser.serialize_none()
	}
}

#[derive(Serialize, Debug)]
pub struct Page<T: Serialize> {
	pub items: Vec<T>,
	#[serde(serialize_with = "optional_uri")]
	pub next: Option<Uri>,
	// TODO: either find some magical way to generate this or change the spec
	#[serde(serialize_with = "optional_uri")]
	pub previous: Option<Uri>,
}

impl <T> Page<T> 
where 
	Reference<T>: From<T>,
	T: Serialize,
{
	pub fn into_references(self) -> Page<Reference<T>> {
		let items = self.items.into_iter()
			.map(Reference::from)
			.collect();

		Page {
			items,
			next: self.next,
			previous: self.previous
		}
	}
}

impl <'t, T> Page<T> 
where 
	Reference<&'t T>: From<&'t T>,
	T: Serialize + 't,
{
	pub fn references(&'t self) -> Page<Reference<&'t T>> {
		let items = self.items.iter()
			.map(Reference::from)
			.collect();

		Page {
			items,
			next: self.next.clone(),
			previous: self.previous.clone(),
		}
	}
}

#[derive(Deserialize, Debug)]
pub struct PaginationOptions<T: PageToken> {
	#[serde(default)]
	pub page: T,
	pub limit: Option<usize>,
}

pub const DEFAULT_PAGE_ITEM_LIMIT: usize = 10;
pub const MAX_PAGE_ITEM_LIMIT: usize = 100;

pub trait PageToken: Default {
	fn start() -> Self { Self::default() }
}