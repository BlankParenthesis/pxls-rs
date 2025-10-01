use bytes::{BytesMut, BufMut};
use sea_orm::sea_query::{Expr, SimpleExpr};
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, Set, StreamTrait, TransactionTrait};
use tokio_stream::StreamExt;

use crate::config::CONFIG;
use crate::database::BoardSpecifier;

use super::entities::*;
use super::{Connection, DbResult, DatabaseError};

use enumset::EnumSetType;
use num_enum::TryFromPrimitive;

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
	pub position: usize,
	pub data: u8,
}

pub struct WriteBuffer {
	pub data: BytesMut,
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
		board: &BoardSpecifier,
		index: i32,
		size: usize,
		connection: &Connection<C>,
	) -> Result<Self, DatabaseError> {
		// NOTE: default mask is NoPlace so that new boards require activation
		// before use.
		let mask = vec![MaskValue::NoPlace as u8; size];
		let initial = vec![0; size];

		connection.create_sector(board, index, mask, initial).await
	}

	pub async fn load<C: ConnectionTrait + TransactionTrait + StreamTrait>(
		board: &BoardSpecifier,
		sector_index: i32,
		connection: &Connection<C>,
	) -> Result<Option<Self>, DatabaseError> {
		connection.get_sector(board, sector_index).await
	}

	pub async fn save<C: ConnectionTrait + TransactionTrait + StreamTrait>(
		&self,
		buffer: SectorBuffer,
		connection: &Connection<C>,
	) -> Result<(), DatabaseError> {
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

impl<C: TransactionTrait + ConnectionTrait + StreamTrait> Connection<C> {
	fn find_sector(board_id: i32, sector_index: i32) -> SimpleExpr {
		board_sector::Column::Sector
			.eq(sector_index)
			.and(board_sector::Column::Board.eq(board_id))
	}
		
	pub async fn create_sector(
		&self,
		board: &BoardSpecifier,
		index: i32,
		mask: Vec<u8>,
		initial: Vec<u8>,
	) -> DbResult<Sector> {

		let new_sector = board_sector::ActiveModel {
			board: Set(board.0),
			sector: Set(index),
			mask: Set(mask),
			initial: Set(initial),
		};

		let sector = board_sector::Entity::insert(new_sector)
			.exec_with_returning(&self.connection).await?;

		self.sector_from_model(sector).await
	}

	pub async fn get_sector(
		&self,
		board: &BoardSpecifier,
		sector_index: i32,
	) -> DbResult<Option<Sector>> {
		let sector = board_sector::Entity::find_by_id((board.0, sector_index))
			.one(&self.connection).await?;

		match sector {
			Some(sector) => self.sector_from_model(sector).await.map(Some),
			None => Ok(None),
		}
	}
	
	async fn sector_from_model(
		&self,
		sector: board_sector::Model,
	) -> DbResult<Sector> {
		let index = sector.sector;
		let board = sector.board;
		let sector_size = sector.initial.len();

		let initial = BytesMut::from(&*sector.initial);
		let mask = BytesMut::from(&*sector.mask);
		let mut colors = initial.clone();
		let mut timestamps = BytesMut::from(&vec![0; sector_size * 4][..]);
		let mut density = BytesMut::from(&vec![0; sector_size * 4][..]);

		let start_position = sector_size as i64 * sector.sector as i64;
		let end_position = start_position + sector_size as i64 - 1;

		let column_timestamp_id_pair = Expr::tuple([
			Expr::col(placement::Column::Timestamp).into(),
			Expr::col(placement::Column::Id).into(),
		]);

		// TODO: look into storing this as indices on the database to skip
		// loading all placements.
		let mut placements = placement::Entity::find()
			.filter(placement::Column::Board.eq(board))
			.filter(placement::Column::Position.between(start_position, end_position))
			.order_by_asc(column_timestamp_id_pair)
			.stream(&self.connection).await?;

		while let Some(placement) = placements.try_next().await? {
			let index = placement.position as usize % sector_size;
			colors[index] = placement.color as u8;
			
			let index4 = index * 4..index * 4 + 4;
			let mut timestamp_slice = &mut timestamps[index4.clone()];
			timestamp_slice.put_u32_le(placement.timestamp as u32);

			let current_density = u32::from_le_bytes(unsafe {
				density[index4.clone()].try_into().unwrap_unchecked()
			});
			let mut density_slice = &mut density[index4];
			density_slice.put_u32_le(current_density + 1);
		}
		
		let initial = WriteBuffer::new(initial);
		let mask = WriteBuffer::new(mask);
		let colors = WriteBuffer::new(colors);
		let timestamps = WriteBuffer::new(timestamps);
		let density = WriteBuffer::new(density);

		Ok(Sector {
			board,
			index,
			initial,
			mask,
			colors,
			timestamps,
			density,
		})
	}
	
	pub async fn write_sector_mask(
		&self,
		board_id: i32,
		sector_index: i32,
		mask: Vec<u8>,
	) -> DbResult<()> {
		board_sector::Entity::update_many()
			.col_expr(board_sector::Column::Mask, mask.into())
			.filter(Self::find_sector(board_id, sector_index))
			.exec(&self.connection).await
			.map(|_| ())
			.map_err(DatabaseError::from)
	}

	pub async fn write_sector_initial(
		&self,
		board_id: i32,
		sector_index: i32,
		initial: Vec<u8>,
	) -> DbResult<()> {
		board_sector::Entity::update_many()
			.col_expr(board_sector::Column::Initial, initial.into())
			.filter(Self::find_sector(board_id, sector_index))
			.exec(&self.connection).await
			.map(|_| ())
			.map_err(DatabaseError::from)
	}
}
