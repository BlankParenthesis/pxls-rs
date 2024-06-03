use std::collections::HashMap;

use serde::Serialize;
use serde_with::skip_serializing_none;

use itertools::Itertools;
use enumset::{EnumSet, EnumSetType};
use warp::http::Uri;

use crate::board::Palette;
use crate::board::Shape;
use crate::filter::response::reference::Reference;
use crate::routes::board_notices::boards::notices::BoardsNotice;
use crate::socket::ServerPacket;

use super::BoardSubscription;

#[derive(Serialize, Debug, Clone)]
pub struct Change<T> {
	pub position: u64,
	pub values: Vec<T>,
}

#[skip_serializing_none]
#[derive(Serialize, Debug, Clone)]
pub struct BoardInfo {
	pub name: Option<String>,
	pub shape: Option<Shape>,
	pub palette: Option<Palette>,
	pub max_pixels_available: Option<u32>,
}

// TODO: rename
#[derive(Debug, EnumSetType)]
pub enum DataType {
	Colors,
	Timestamps,
	Initial,
	Mask,
	Info,
}

impl From<BoardSubscription> for Option<DataType> {
	fn from(subscription: BoardSubscription) -> Self {
		match subscription {
			BoardSubscription::DataColors => Some(DataType::Colors),
			BoardSubscription::DataTimestamps => Some(DataType::Timestamps),
			BoardSubscription::DataInitial => Some(DataType::Initial),
			BoardSubscription::DataMask => Some(DataType::Mask),
			BoardSubscription::Info => Some(DataType::Info),
			_ => None,
		}
	}
}

#[skip_serializing_none]
#[derive(Serialize, Debug, Default, Clone)]
pub struct BoardData {
	colors: Option<Box<[Change<u8>]>>,
	timestamps: Option<Box<[Change<u32>]>>,
	initial: Option<Box<[Change<u8>]>>,
	mask: Option<Box<[Change<u8>]>>,
}

impl BoardData {
	pub fn builder() -> BoardUpdateBuilder {
		BoardUpdateBuilder::default()
	}

	fn is_empty(&self) -> bool {
		self.colors.is_none()
		&& self.timestamps.is_none()
		&& self.initial.is_none()
		&& self.mask.is_none()
	}
}

#[derive(Debug, Default)]
pub struct BoardUpdateBuilder {
	colors: Option<Vec<Change<u8>>>,
	timestamps: Option<Vec<Change<u32>>>,
	initial: Option<Vec<Change<u8>>>,
	mask: Option<Vec<Change<u8>>>,
	info: Option<BoardInfo>,
}

impl BoardUpdateBuilder {
	pub fn new() -> Self {
		Self::default()
	}

	pub fn colors(mut self, colors: Vec<Change<u8>>) -> Self {
		assert!(self.colors.replace(colors).is_none());
		self
	}

	pub fn timestamps(mut self, timestamps: Vec<Change<u32>>) -> Self {
		assert!(self.timestamps.replace(timestamps).is_none());
		self
	}

	pub fn initial(mut self, initial: Vec<Change<u8>>) -> Self {
		assert!(self.initial.replace(initial).is_none());
		self
	}

	pub fn mask(mut self, mask: Vec<Change<u8>>) -> Self {
		assert!(self.mask.replace(mask).is_none());
		self
	}

	pub fn info(mut self, info: BoardInfo) -> Self {
		assert!(self.info.replace(info).is_none());
		self
	}

	pub fn merge(&mut self, other: Self) {
		if let Some(mut o) = other.colors {
			if let Some(ref mut s) = self.colors {
				s.append(&mut o);
			} else {
				self.colors = Some(o);
			}
		}

		if let Some(mut o) = other.timestamps {
			if let Some(ref mut s) = self.timestamps {
				s.append(&mut o);
			} else {
				self.timestamps = Some(o);
			}
		}

		if let Some(mut o) = other.initial {
			if let Some(ref mut s) = self.initial {
				s.append(&mut o);
			} else {
				self.initial = Some(o);
			}
		}

		if let Some(mut o) = other.mask {
			if let Some(ref mut s) = self.mask {
				s.append(&mut o);
			} else {
				self.mask = Some(o);
			}
		}

		if let Some(o) = other.info {
			if let Some(ref s) = self.info {
				todo!("double info") // this should be detected and prevented
			} else {
				self.info = Some(o);
			}
		}
	}

	pub fn build_combinations(self) -> HashMap<EnumSet<DataType>, Packet> {
		let mut combinations = HashMap::new();

		let colors = self.colors.map(Vec::into_boxed_slice);
		let timestamps = self.timestamps.map(Vec::into_boxed_slice);
		let initial = self.initial.map(Vec::into_boxed_slice);
		let mask = self.mask.map(Vec::into_boxed_slice);

		let mut available_types = vec![];

		if colors.is_some() {
			available_types.push(DataType::Colors);
		}

		if timestamps.is_some() {
			available_types.push(DataType::Timestamps);
		}

		if initial.is_some() {
			available_types.push(DataType::Initial);
		}

		if mask.is_some() {
			available_types.push(DataType::Mask);
		}

		if self.info.is_some() {
			available_types.push(DataType::Info);
		}

		for combination in available_types.into_iter().powerset().skip(1) {
			let mut info = None;
			let mut data = BoardData::default();

			for datatype in combination.iter() {
				match datatype {
					DataType::Colors => data.colors = colors.clone(),
					DataType::Timestamps => data.timestamps = timestamps.clone(),
					DataType::Initial => data.initial = mask.clone(),
					DataType::Mask => data.mask = mask.clone(),
					DataType::Info => info = self.info.clone(),
				}
			}

			let data = if data.is_empty() { None } else { Some(data) };

			let key = combination.into_iter().collect();
			combinations.insert(key, Packet::BoardUpdate { info, data });
		}

		combinations
	}
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
	BoardNoticeCreated {
		notice: Reference<BoardsNotice>,
	},
	BoardNoticeUpdated {
		notice: Reference<BoardsNotice>,
	},
	BoardNoticeDeleted {
		#[serde(with = "http_serde::uri")]
		notice: Uri,
	},
}

impl From<&Packet> for BoardSubscription {
	fn from(event: &Packet) -> Self {
		match event {
			Packet::BoardUpdate { .. } => BoardSubscription::DataColors,
			Packet::PixelsAvailable { .. } => BoardSubscription::Cooldown,
			Packet::BoardNoticeCreated { .. } => BoardSubscription::Notices,
			Packet::BoardNoticeUpdated { .. } => BoardSubscription::Notices,
			Packet::BoardNoticeDeleted { .. } => BoardSubscription::Notices,
		}
	}
}

impl ServerPacket for Packet {}