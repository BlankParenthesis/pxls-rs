use std::collections::HashMap;

use serde::Serialize;
use serde_with::skip_serializing_none;

use itertools::Itertools;
use enumset::{EnumSet, EnumSetType};

use crate::board::Palette;
use crate::board::Shape;
use crate::socket::Extension;

#[derive(Serialize, Debug, Clone)]
pub struct Change<T> {
	pub position: u64,
	pub values: Vec<T>,
}

#[derive(Serialize, Debug, Clone)]
#[skip_serializing_none]
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

impl From<Extension> for Option<DataType> {
	fn from(extension: Extension) -> Self {
		match extension {
			Extension::Core => Some(DataType::Colors),
			Extension::BoardTimestamps => Some(DataType::Timestamps),
			Extension::BoardInitial => Some(DataType::Initial),
			Extension::BoardMask => Some(DataType::Mask),
			Extension::BoardLifecycle => Some(DataType::Info),
			_ => None,
		}
	}
}

#[derive(Serialize, Debug, Default, Clone)]
#[skip_serializing_none]
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

#[derive(Serialize, Debug, Clone)]
#[skip_serializing_none]
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
			Packet::BoardUpdate { .. } => Extension::Core,
			Packet::PixelsAvailable { .. } => Extension::Core,
			Packet::Ready => Extension::Core,
		}
	}
}
