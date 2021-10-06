use diesel::QueryResult;
use diesel::prelude::*;
use actix_web::web::{BytesMut, BufMut};

use crate::database::{model, schema, Connection};
use crate::objects::MaskValue;

#[derive(PartialEq)]
pub enum SectorBuffer {
	Colors,
	Timestamps,
	Initial,
	Mask,
}

pub struct BoardSector {
	board: i32,
	index: i32,
	pub colors: BytesMut,
	pub timestamps: BytesMut,
	pub mask: BytesMut,
	pub initial: BytesMut,
}

impl BoardSector {
	pub fn new(
		board: i32,
		index: i32,
		size: usize,
		connection: &Connection,
	) -> QueryResult<Self> {
		// NOTE: default mask is NoPlace so that new boards require activation
		// before use.
		let mask = vec![MaskValue::NoPlace as u8; size];
		let initial = vec![0; size];

		let new_sector = model::BoardSector {
			board,
			index,
			mask,
			initial,
		};

		diesel::insert_into(schema::board_sector::table)
			.values(&new_sector)
			.execute(connection)?;

		Self::from_model(new_sector, connection)
	}

	pub fn load(
		board_id: i32,
		sector_index: i32,
		connection: &Connection,
	) -> QueryResult<Option<Self>> {
		let sector = schema::board_sector::table
			.find((board_id, sector_index))
			.load::<model::BoardSector>(connection)?
			.pop();

		if let Some(sector) = sector {
			Ok(Some(Self::from_model(sector, connection)?))
		} else {
			Ok(None)
		}
	}

	pub fn save(
		&self,
		connection: &Connection,
		buffer: Option<&SectorBuffer>,
	) -> QueryResult<()> {
		match buffer {
			Some(SectorBuffer::Colors) => unimplemented!(),
			Some(SectorBuffer::Timestamps) => unimplemented!(),
			Some(SectorBuffer::Initial) => {
				diesel::update(schema::board_sector::table)
					.set(schema::board_sector::initial.eq(&*self.initial))
					.filter(schema::board_sector::index.eq(self.index)
						.and(schema::board_sector::board.eq(self.board)))
					.execute(connection)
					.map(|_| ())
			},
			Some(SectorBuffer::Mask) => {
				diesel::update(schema::board_sector::table)
					.set(schema::board_sector::mask.eq(&*self.mask))
					.filter(schema::board_sector::index.eq(self.index)
						.and(schema::board_sector::board.eq(self.board)))
					.execute(connection)
					.map(|_| ())
			},
			None => {
				diesel::update(schema::board_sector::table)
					.set((
						schema::board_sector::initial.eq(&*self.initial),
						schema::board_sector::mask.eq(&*self.mask),
					))
					.filter(schema::board_sector::index.eq(self.index)
						.and(schema::board_sector::board.eq(self.board)))
					.execute(connection)
					.map(|_| ())
			},
		}
	}


	fn from_model(
		sector: model::BoardSector,
		connection: &Connection,
	) -> QueryResult<Self> {
		let index = sector.index;
		let board = sector.board;
		let sector_size = sector.initial.len();

		let initial = BytesMut::from(&*sector.initial);
		let mask = BytesMut::from(&*sector.mask);
		let mut colors = initial.clone();
		let mut timestamps = BytesMut::from(&vec![0; sector_size * 4][..]);

		let start_position = sector_size as i64 * sector.index as i64;
		let end_position = start_position + sector_size as i64 - 1;

		// TODO: maybe this will be possible in qsl one day…
		// until then, maybe there's a non-nested way to do this.
		let placements = diesel::sql_query("
			SElECT DISTINCT ON (position) * FROM (
				SELECT * FROM placement
				WHERE board = $1
				AND position BETWEEN $2 AND $3
				ORDER BY timestamp DESC, id DESC
			) AS ordered")
			.bind::<diesel::sql_types::Int4, _>(sector.board)
			.bind::<diesel::sql_types::Int8, _>(start_position)
			.bind::<diesel::sql_types::Int8, _>(end_position)
			.load::<model::Placement>(connection)?;

		for placement in placements {
			let index = placement.position as usize;
			colors[index] = placement.color as u8;
			let mut timestamp_slice = &mut timestamps[index * 4..index * 4 + 4];
			timestamp_slice.put_u32_le(placement.timestamp as u32);
		}

		Ok(Self { board, index, initial, mask, colors, timestamps })
	}
}
