use bytes::{BytesMut, BufMut};
use sea_orm::{sea_query::{Query, self, Expr}, Order, Set, EntityTrait, ColumnTrait, QueryFilter, Iden, ConnectionTrait};

use crate::{
	database::boards::{entities::*, DbResult},
	board::board::MaskValue,
};

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
	board: i32,
	index: i32,
	pub colors: BytesMut,
	pub timestamps: BytesMut,
	pub mask: BytesMut,
	pub initial: BytesMut,
	// TODO: maybe a density buffer for how many placements there have been
}

impl Sector {
	pub async fn new<Connection: ConnectionTrait>(
		board: i32,
		index: i32,
		size: usize,
		connection: &Connection,
	) -> DbResult<Self> {
		// NOTE: default mask is NoPlace so that new boards require activation
		// before use.
		let mask = vec![MaskValue::NoPlace as u8; size];
		let initial = vec![0; size];

		let new_sector = board_sector::ActiveModel {
			board: Set(board),
			sector: Set(index),
			mask: Set(mask),
			initial: Set(initial),
		};

		let sector = board_sector::Entity::insert(new_sector)
			.exec_with_returning(connection).await?;

		Self::from_model(sector, connection).await
	}

	pub async fn load<Connection: ConnectionTrait>(
		board_id: i32,
		sector_index: i32,
		connection: &Connection,
	) -> DbResult<Option<Self>> {
		let sector = board_sector::Entity::find_by_id((board_id, sector_index))
			.one(connection).await?;

		if let Some(sector) = sector {
			Ok(Some(Self::from_model(sector, connection).await?))
		} else {
			Ok(None)
		}
	}

	pub async fn save<Connection: ConnectionTrait>(
		&self,
		connection: &Connection,
		buffer: Option<&SectorBuffer>,
	) -> DbResult<()> {
		let find_this_sector = board_sector::Column::Sector
			.eq(self.index)
			.and(board_sector::Column::Board.eq(self.board));

		match buffer {
			Some(SectorBuffer::Colors) => unimplemented!(),
			Some(SectorBuffer::Timestamps) => unimplemented!(),
			Some(SectorBuffer::Initial) => {
				board_sector::Entity::update_many()
					.col_expr(board_sector::Column::Initial, self.initial.to_vec().into())
					.filter(find_this_sector)
					.exec(connection).await
					.map(|_| ())
			},
			Some(SectorBuffer::Mask) => {
				board_sector::Entity::update_many()
					.col_expr(board_sector::Column::Mask, self.mask.to_vec().into())
					.filter(find_this_sector)
					.exec(connection).await
					.map(|_| ())
			},
			None => {
				board_sector::Entity::update_many()
					.col_expr(board_sector::Column::Initial, self.initial.to_vec().into())
					.col_expr(board_sector::Column::Mask, self.mask.to_vec().into())
					.filter(find_this_sector)
					.exec(connection).await
					.map(|_| ())
			},
		}
	}

	async fn from_model<Connection: ConnectionTrait>(
		sector: board_sector::Model,
		connection: &Connection,
	) -> DbResult<Self> {
		let index = sector.sector;
		let board = sector.board;
		let sector_size = sector.initial.len();

		let initial = BytesMut::from(&*sector.initial);
		let mask = BytesMut::from(&*sector.mask);
		let mut colors = initial.clone();
		let mut timestamps = BytesMut::from(&vec![0; sector_size * 4][..]);

		let start_position = sector_size as i64 * sector.sector as i64;
		let end_position = start_position + sector_size as i64 - 1;

		#[derive(Iden)]
		struct Inner;

		let placements = placement::Entity::find()
			.filter(placement::Column::Board.eq(board))
			.filter(placement::Column::Position.between(start_position, end_position))
			.filter(placement::Column::Id.in_subquery(
				Query::select()
					.from_as(placement::Entity, Inner)
					.column((Inner, placement::Column::Id))
					.and_where(
						Expr::col((placement::Entity, placement::Column::Position))
							.equals((Inner, placement::Column::Position))
					)
					.order_by((Inner, placement::Column::Timestamp), Order::Desc)
					.order_by((Inner, placement::Column::Id), Order::Desc)
					.limit(1)
					.to_owned()
			))
			.all(connection).await?;

		for placement in placements {
			let index = placement.position as usize;
			colors[index] = placement.color as u8;
			let mut timestamp_slice = &mut timestamps[index * 4..index * 4 + 4];
			timestamp_slice.put_u32_le(placement.timestamp as u32);
		}

		Ok(Self {
			board,
			index,
			initial,
			mask,
			colors,
			timestamps,
		})
	}
}
