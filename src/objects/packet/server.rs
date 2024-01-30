use serde::Serialize;
use serde_with::skip_serializing_none;

use itertools::Itertools;
use enum_map::{Enum, EnumMap};
use enumset::{EnumSet, EnumSetType};

use crate::objects::{Extension, Palette, CachedVecShape};

#[derive(Serialize, Debug, Clone)]
pub struct Change<T> {
	pub position: u64,
	pub values: Vec<T>,
}

#[skip_serializing_none]
#[derive(Serialize, Debug, Clone)]
pub struct BoardInfo {
	pub name: Option<String>,
	pub shape: Option<CachedVecShape>,
	pub palette: Option<Palette>,
	pub max_pixels_available: Option<u32>,
}

// TODO: this is the same as SectorBuffer in board_sectors — deduplicate?
#[derive(Debug, EnumSetType)]
enum DataType {
	Colors = 0,
	Timestamps = 1,
	Initial = 2,
	Mask = 3,
}

#[derive(Debug, Enum, Clone, Copy, PartialEq, Eq)]
pub enum BoardDataCombination {
	Colors,
	Timestamps,
	Initial,
	Mask,
	ColorsTimestamps,
	ColorsInitial,
	ColorsMask,
	TimestampsInitial,
	TimestampsMask,
	InitialMask,
	ColorsTimestampsInitial,
	ColorsTimestampsMask,
	ColorsInitialMask,
	TimestampsInitialMask,
	All,
}

impl From<EnumSet<DataType>> for BoardDataCombination {
	fn from(set: EnumSet<DataType>) -> Self {
		// bit 0 is colors
		// bit 1 is timestamps
		// bit 2 is initial
		// bit 3 is mask
		// TODO: this is untested but definitely should be.
		match set.as_u8() {
			0b1000 => Self::Colors,
			0b0100 => Self::Timestamps,
			0b0010 => Self::Initial,
			0b0001 => Self::Mask,
			0b1100 => Self::ColorsTimestamps,
			0b1010 => Self::ColorsInitial,
			0b1001 => Self::ColorsMask,
			0b0110 => Self::TimestampsInitial,
			0b0101 => Self::TimestampsMask,
			0b0011 => Self::InitialMask,
			0b1110 => Self::ColorsTimestampsInitial,
			0b1101 => Self::ColorsTimestampsMask,
			0b1011 => Self::ColorsInitialMask,
			0b0111 => Self::TimestampsInitialMask,
			0b1111 => Self::All,
			_ => panic!(),
		}
	}
}

impl From<EnumSet<Extension>> for BoardDataCombination {
	fn from(extensions: EnumSet<Extension>) -> Self {
		let mut data_types = EnumSet::empty();

		if extensions.contains(Extension::Core) {
			data_types.insert(DataType::Colors);
		}

		if extensions.contains(Extension::BoardTimestamps) {
			data_types.insert(DataType::Timestamps);
		}

		if extensions.contains(Extension::BoardInitial) {
			data_types.insert(DataType::Initial);
		}

		if extensions.contains(Extension::BoardMask) {
			data_types.insert(DataType::Mask);
		}
		
		Self::from(data_types)
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
	pub fn builder() -> BoardDataBuilder {
		BoardDataBuilder::default()
	}
}

#[derive(Debug, Default)]
pub struct BoardDataBuilder {
	colors: Option<Vec<Change<u8>>>,
	timestamps: Option<Vec<Change<u32>>>,
	initial: Option<Vec<Change<u8>>>,
	mask: Option<Vec<Change<u8>>>,
}

impl BoardDataBuilder {
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

	pub fn build_combinations(self) -> EnumMap<BoardDataCombination, BoardData> {
		let mut combinations = EnumMap::default();

		let colors = self.colors.map(Vec::into_boxed_slice);
		let timestamps = self.timestamps.map(Vec::into_boxed_slice);
		let initial = self.initial.map(Vec::into_boxed_slice);
		let mask = self.mask.map(Vec::into_boxed_slice);

		let mut available_types = vec![];

		if colors.is_some() {
			available_types.push(DataType::Colors)
		}

		if timestamps.is_some() {
			available_types.push(DataType::Timestamps)
		}

		if initial.is_some() {
			available_types.push(DataType::Initial)
		}

		if mask.is_some() {
			available_types.push(DataType::Mask)
		}

		for combination in available_types.into_iter().powerset().skip(1) {
			let mut data = BoardData::default();

			for datatype in combination.iter() {
				match datatype {
					DataType::Colors => data.colors = colors.clone(),
					DataType::Timestamps => data.timestamps = timestamps.clone(),
					DataType::Initial => data.initial = mask.clone(),
					DataType::Mask => data.mask = mask.clone(),
				}
			}

			let combination = combination.into_iter().collect::<EnumSet<_>>();
			let key = BoardDataCombination::from(combination);
			combinations[key] = data;
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
