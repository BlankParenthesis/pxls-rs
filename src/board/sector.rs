use bytes::{BufMut, BytesMut};
use enumset::EnumSetType;
use num_enum::TryFromPrimitive;
use sea_orm::{ConnectionTrait, TransactionTrait, StreamTrait};

mod cache;
mod access;

use crate::{config::CONFIG, database::{BoardsConnectionGeneric, BoardsDatabaseError}};

pub use cache::{BufferedSectorCache, SectorCache, CompressedSector};
pub use access::{SectorAccessor, IoError};

#[derive(TryFromPrimitive)]
#[repr(u8)]
pub enum MaskValue {
	NoPlace = 0,
	Place = 1,
	Adjacent = 2,
}

#[derive(EnumSetType)]
pub enum SectorBuffer {
	Colors,
	Timestamps,
	Initial,
	Mask,
	Density,
}

impl SectorBuffer {
	pub fn size(&self) -> usize {
		match self {
			SectorBuffer::Colors => 1,
			SectorBuffer::Timestamps => 4,
			SectorBuffer::Initial => 1,
			SectorBuffer::Mask => 1,
			SectorBuffer::Density => 4,
		}
	}
}

pub enum BufferRead {
	Delta(Vec<Change>),
	Full(BytesMut),
}

pub struct Change {
	position: usize,
	data: u8,
}

pub struct WriteBuffer {
	data: BytesMut,
	recent_changes: Option<Vec<Change>>,
}

impl WriteBuffer {
	pub fn new(data: BytesMut) -> Self {
		Self { data, recent_changes: None }
	}
	
	pub fn write(&mut self, position: usize, data: u8) {
		self.data[position] = data;
				
		if let Some(changes) = self.recent_changes.as_mut() {
			if changes.len() < CONFIG.buffered_readback_limit {
				changes.push(Change { position, data })
			} else {
				self.recent_changes = None;
			}
		}
	}
	
	pub fn write_u32(&mut self, position: usize, data: u32) {
		let start = position * 4;
		let end = start + 4;
		let range = start..end;
		self.data[range].as_mut().put_u32_le(data);
		
		if let Some(changes) = self.recent_changes.as_mut() {
			if changes.len() < CONFIG.buffered_readback_limit {
				let bytes = data.to_le_bytes();
				changes.push(Change { position: start, data: bytes[0] });
				changes.push(Change { position: start + 1, data: bytes[1] });
				changes.push(Change { position: start + 2, data: bytes[2] });
				changes.push(Change { position: start + 3, data: bytes[3] });
			} else {
				self.recent_changes = None;
			}
		}
	}
	
	pub fn read(&self, position: usize) -> u8 {
		self.data[position]
	}
	
	pub fn read_u32(&self, position: usize) -> u32 {
		let start = position * 4;
		let end = start + 4;
		let range = start..end;
		u32::from_le_bytes(self.data[range].try_into().unwrap())
	}
	
	pub fn readback(&mut self) -> BufferRead {
		match self.recent_changes.as_mut() {
			Some(changes) => {
				let new = Vec::with_capacity(changes.len());
				BufferRead::Delta(std::mem::replace(changes, new))
			},
			None => {
				self.recent_changes = Some(vec![]);
				BufferRead::Full(self.data.clone())
			},
		}
	}
}

pub struct Sector {
	pub board: i32,
	pub index: i32,
	pub colors: WriteBuffer,
	pub timestamps: WriteBuffer,
	pub mask: WriteBuffer,
	pub initial: WriteBuffer,
	// the number of placements on a position
	pub density: WriteBuffer,
}

impl Sector {
	pub async fn new<C: ConnectionTrait + TransactionTrait + StreamTrait>(
		board_id: i32,
		index: i32,
		size: usize,
		connection: &BoardsConnectionGeneric<C>,
	) -> Result<Self, BoardsDatabaseError> {
		// NOTE: default mask is NoPlace so that new boards require activation
		// before use.
		let mask = vec![MaskValue::NoPlace as u8; size];
		let initial = vec![0; size];

		connection.create_sector(board_id, index, mask, initial).await
	}

	pub async fn load<C: ConnectionTrait + TransactionTrait + StreamTrait>(
		board_id: i32,
		sector_index: i32,
		connection: &BoardsConnectionGeneric<C>,
	) -> Result<Option<Self>, BoardsDatabaseError> {
		connection.get_sector(board_id, sector_index).await
	}

	pub async fn save<C: ConnectionTrait + TransactionTrait + StreamTrait>(
		&self,
		buffer: SectorBuffer,
		connection: &BoardsConnectionGeneric<C>,
	) -> Result<(), BoardsDatabaseError> {
		match buffer {
			SectorBuffer::Colors => unimplemented!(),
			SectorBuffer::Timestamps => unimplemented!(),
			SectorBuffer::Density => unimplemented!(),
			SectorBuffer::Initial => {
				connection.write_sector_initial(
					self.board,
					self.index,
					self.initial.data.to_vec(),
				).await
			},
			SectorBuffer::Mask => {
				connection.write_sector_mask(
					self.board,
					self.index,
					self.mask.data.to_vec(),
				).await
			},
		}
	}
}
