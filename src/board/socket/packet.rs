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
use crate::routes::placement_statistics::users::PlacementColorStatistics;
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
				// FIXME: this should be detected and prevented
				todo!("double info")
			} else {
				self.info = Some(o);
			}
		}
	}
	
	/// condense adjacent changes into a single entry
	fn compress_changes<T: Copy + Default>(changes: Vec<Change<T>>) -> Vec<Change<T>> {
		// merged changes, sorted by position
		let mut merged = vec![] as Vec<Change<T>>;
		
		// 1. find the first and last intersections
		// 2. remove that range from the existing merged changes
		// 3. take the first and last of that removed section
		// 4. prepend the first, append the last
		// 5. insert the new change at the correct index
		for mut change in changes {
			let start = change.position;
			let end = start + change.values.len() as u64;
			
			let first_intersection = merged.iter()
				.position(|change| {
					let change_end = change.position + change.values.len() as u64;
					start <= change_end
				})
				.unwrap_or(merged.len());
			
			let last_intersection = merged.iter()
				.rev()
				.position(|change| {
					let change_start = change.position;
					change_start <= end
				})
				.map(|p| merged.len() - p)
				.unwrap_or(0);
			
			let inserection_range = if first_intersection < last_intersection {
				first_intersection..last_intersection
			} else {
				0..0
			};
			let mut replaced_changes = merged.splice(inserection_range, []);
			
			if let Some(mut first) = replaced_changes.next() {
				let count_prepended = start.saturating_sub(first.position) as usize;
				let new_values_end = change.values.len() + count_prepended; 
				if first.values.len() < new_values_end {
					first.values.append(&mut change.values);
				} else {
					first.values[count_prepended..new_values_end]
						.copy_from_slice(&change.values);
				}
				change = first;
			}
			if let Some(last) = replaced_changes.last() {
				let offset = (end - last.position) as usize;
				if offset < last.values.len() {
					change.values.extend_from_slice(&last.values[offset..]);
				}
			}
			
			let insert_index = merged.binary_search_by_key(&change.position, |c| c.position)
				.expect_err("Failed to properly remove overlap when pruning ranges");
		
			merged.insert(insert_index, change);
		}
		
		merged
	}
	
	/// condense all fields to a minimal form
	pub fn minify(&mut self) {
		if let Some(colors) = self.colors.take() {
			self.colors = Some(Self::compress_changes(colors));
		}

		if let Some(timestamps) = self.timestamps.take() {
			self.timestamps = Some(Self::compress_changes(timestamps));
		}

		if let Some(initial) = self.initial.take() {
			self.initial = Some(Self::compress_changes(initial));
		}

		if let Some(mask) = self.mask.take() {
			self.mask = Some(Self::compress_changes(mask));
		}
	}

	pub fn build_combinations(self) -> HashMap<EnumSet<DataType>, Packet> {
		let mut combinations = HashMap::new();

		let colors = self.colors.map(Vec::into_boxed_slice);
		let timestamps = self.timestamps.map(Vec::into_boxed_slice);
		let initial = self.initial.map(Vec::into_boxed_slice);
		let mask = self.mask.map(Vec::into_boxed_slice);

		let mut used_types = vec![];

		if colors.is_some() {
			used_types.push(DataType::Colors);
		}

		if timestamps.is_some() {
			used_types.push(DataType::Timestamps);
		}

		if initial.is_some() {
			used_types.push(DataType::Initial);
		}

		if mask.is_some() {
			used_types.push(DataType::Mask);
		}

		if self.info.is_some() {
			used_types.push(DataType::Info);
		}

		for combination in EnumSet::<DataType>::all().iter().powerset().skip(1) {
			if !used_types.iter().any(|t| combination.iter().contains(t)) {
				// if the combination does not contain any used type,
				// it is empty and can be  skipped
				continue;
			}

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
	BoardStatsUpdated {
		stats: PlacementColorStatistics,
	}
}

impl From<&Packet> for BoardSubscription {
	fn from(event: &Packet) -> Self {
		match event {
			Packet::BoardUpdate { .. } => BoardSubscription::DataColors,
			Packet::PixelsAvailable { .. } => BoardSubscription::Cooldown,
			Packet::BoardNoticeCreated { .. } => BoardSubscription::Notices,
			Packet::BoardNoticeUpdated { .. } => BoardSubscription::Notices,
			Packet::BoardNoticeDeleted { .. } => BoardSubscription::Notices,
			Packet::BoardStatsUpdated { .. } => BoardSubscription::Statistics,
		}
	}
}

impl ServerPacket for Packet {}
