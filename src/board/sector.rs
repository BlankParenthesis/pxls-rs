use bytes::BytesMut;
use num_enum::TryFromPrimitive;
use sea_orm::{DbErr, ConnectionTrait, TransactionTrait};

mod cache;
mod access;

use crate::database::BoardsConnectionGeneric;

pub use cache::SectorCache;
pub use access::SectorAccessor;

#[derive(TryFromPrimitive)]
#[repr(u8)]
pub enum MaskValue {
	NoPlace = 0,
	Place = 1,
	Adjacent = 2,
}

#[derive(PartialEq, Clone, Copy)]
pub enum SectorBuffer {
	Colors,
	Timestamps,
	Initial,
	Mask,
}

impl SectorBuffer {
	pub fn size(&self) -> usize {
		match self {
			SectorBuffer::Colors => 1,
			SectorBuffer::Timestamps => 4,
			SectorBuffer::Initial => 1,
			SectorBuffer::Mask => 1,
		}
	}
}

pub struct Sector {
	pub board: i32,
	pub index: i32,
	pub colors: BytesMut,
	pub timestamps: BytesMut,
	pub mask: BytesMut,
	pub initial: BytesMut,
	// TODO: maybe a density buffer for how many placements there have been
}

impl Sector {
	pub async fn new<C: ConnectionTrait + TransactionTrait>(
		board_id: i32,
		index: i32,
		size: usize,
		connection: &BoardsConnectionGeneric<C>,
	) -> Result<Self, DbErr> {
		// NOTE: default mask is NoPlace so that new boards require activation
		// before use.
		let mask = vec![MaskValue::NoPlace as u8; size];
		let initial = vec![0; size];

		connection.create_sector(board_id, index, mask, initial).await
	}

	pub async fn load<C: ConnectionTrait + TransactionTrait>(
		board_id: i32,
		sector_index: i32,
		connection: &BoardsConnectionGeneric<C>,
	) -> Result<Option<Self>, DbErr> {
		connection.get_sector(board_id, sector_index).await
	}

	pub async fn save<C: ConnectionTrait + TransactionTrait>(
		&self,
		buffer: SectorBuffer,
		connection: &BoardsConnectionGeneric<C>,
	) -> Result<(), DbErr> {
		match buffer {
			SectorBuffer::Colors => unimplemented!(),
			SectorBuffer::Timestamps => unimplemented!(),
			SectorBuffer::Initial => {
				connection.write_sector_initial(
					self.board,
					self.index,
					self.initial.to_vec(),
				).await
			},
			SectorBuffer::Mask => {
				connection.write_sector_initial(
					self.board,
					self.index,
					self.mask.to_vec(),
				).await
			},
		}
	}
}
