use serde::{Serialize, Deserialize};
use actix_web::web::BufMut;
use std::time::{SystemTime, UNIX_EPOCH};
use std::io::{Write, Seek, SeekFrom};
use http::Uri;
use num_derive::FromPrimitive;    
use num_traits::FromPrimitive;
use diesel::types::Record;
use diesel::prelude::*;
use diesel::Connection as DConnection;

use crate::database::{Connection, model, schema};
use crate::objects::{
	Color,
	Reference,
	Palette,
	User,
	BinaryPatch,
	Shape,
	VecShape,
	SectorCache,
	SectorBuffer,
};


#[derive(Serialize, Debug)]
pub struct BoardInfo {
	pub name: String,
	pub created_at: u64,
	pub shape: VecShape,
	pub palette: Palette,
}

#[derive(Deserialize, Debug)]
pub struct BoardInfoPost {
	pub name: String,
	pub shape: VecShape,
	pub palette: Palette,
}


#[derive(Deserialize, Debug)]
pub struct BoardInfoPatch {
	pub name: Option<String>,
	pub shape: Option<VecShape>,
	pub palette: Option<Palette>,
}

pub struct Board {
	pub id: i32,
	pub info: BoardInfo,
	pub sectors: SectorCache,
}

#[derive(FromPrimitive)]
pub enum MaskValue {
	NoPlace = 0,
	Place = 1,
	Adjacent = 2,
}

#[derive(Debug)]
pub enum PlaceError {
	UnknownMaskValue,
	Unplacable,
	InvalidColor,
	NoOp,
	Cooldown,
	OutOfBounds,
}

impl std::error::Error for PlaceError {}

impl std::fmt::Display for PlaceError {
	fn fmt(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
		match self {
			PlaceError::UnknownMaskValue => write!(formatter, "Unknown mask value"),
			PlaceError::Unplacable => write!(formatter, "Position is unplacable"),
			PlaceError::InvalidColor => write!(formatter, "No such color on palette"),
			PlaceError::NoOp => write!(formatter, "Placement would have no effect"),
			PlaceError::Cooldown => write!(formatter, "No placements available"),
			PlaceError::OutOfBounds => write!(formatter, "Position is out of bounds"),
		}
	}
}

impl From<PlaceError> for actix_web::Error {
	fn from(place_error: PlaceError) -> Self {
		match place_error {
			PlaceError::UnknownMaskValue => actix_web::error::ErrorInternalServerError(place_error),
			PlaceError::Unplacable => actix_web::error::ErrorForbidden(place_error),
			PlaceError::InvalidColor => actix_web::error::ErrorUnprocessableEntity(place_error),
			PlaceError::NoOp => actix_web::error::ErrorConflict(place_error),
			PlaceError::Cooldown => actix_web::error::ErrorTooManyRequests(place_error),
			PlaceError::OutOfBounds => actix_web::error::ErrorNotFound(place_error),
		}
	}
}

impl Board {
	pub fn create(
		info: BoardInfoPost, 
		connection: &Connection
	) -> QueryResult<Self> {
		let now = SystemTime::now()
			.duration_since(UNIX_EPOCH).unwrap()
			.as_secs();

		let new_board = diesel::insert_into(schema::board::table)
			.values(model::NewBoard {
				name: info.name,
				created_at: now as i64,
				shape: info.shape.into(),
			})
			.get_result::<model::Board>(connection)?;

		crate::objects::color::replace_palette(
			&info.palette,
			new_board.id,
			connection,
		)?;

		Self::load(new_board, connection)
	}

	// TODO: proper error type
	// TODO: notify the damn websocket server.
	// Poor websocket server never gets told about anything 😭.
	pub fn try_patch_initial(
		&mut self,
		patch: &BinaryPatch,
		connection: &Connection,
	) -> Result<(), &'static str> {
		// TODO: check bounds
		let mut sector_data = self.sectors
			.access(SectorBuffer::Initial, connection);

		sector_data.seek(SeekFrom::Start(patch.start))
			.map_err(|_| "invalid start position")?;

		sector_data.write(&*patch.data)
			.map_err(|_| "write error")?;

		Ok(())
	}

	pub fn try_patch_mask(
		&mut self,
		patch: &BinaryPatch,
		connection: &Connection
	) -> Result<(), &'static str> {
		let mut sector_data = self.sectors
			.access(SectorBuffer::Mask, connection);

		sector_data.seek(SeekFrom::Start(patch.start))
			.map_err(|_| "invalid start position")?;

		sector_data.write(&*patch.data)
			.map_err(|_| "write error")?;

		Ok(())
	}

	pub fn update_info(
		&mut self, 
		info: BoardInfoPatch, 
		connection: &Connection,
	) -> QueryResult<()> {
		assert!(info.name.is_some() || info.palette.is_some() || info.shape.is_some());

		connection.transaction::<_, diesel::result::Error, _>(|| {
			if let Some(name) = &info.name {
				diesel::update(schema::board::table)
					.set(schema::board::name.eq(name))
					.filter(schema::board::id.eq(self.id))
					.execute(connection)?;
			}

			if let Some(palette) = &info.palette {
				crate::objects::color::replace_palette(palette, self.id, connection)?;
			}

			if let Some(shape) = &info.shape {
				// TODO: try and preserve data.

				diesel::update(schema::board::table)
					.set(schema::board::shape.eq(
						serde_json::to_value(shape).unwrap()
					))
					.filter(schema::board::id.eq(self.id))
					.execute(connection)?;

				diesel::delete(schema::board_sector::table)
					.filter(schema::board_sector::board.eq(self.id))
					.execute(connection)?;
			}

			Ok(())
		})?;

		if let Some(name) = info.name {
			self.info.name = name;
		}

		if let Some(palette) = info.palette {
			self.info.palette = palette;
		}

		if let Some(shape) = info.shape {
			self.info.shape = shape;

			self.sectors = SectorCache::new(
				self.id,
				self.info.shape.sector_count(),
				self.info.shape.sector_size(),
			)
		}

		Ok(())
	}

	pub fn delete(self, connection: &Connection) -> QueryResult<()> {
		connection.transaction(|| {
			diesel::delete(schema::board_sector::table)
				.filter(schema::board_sector::board.eq(self.id))
				.execute(connection)?;

			diesel::delete(schema::placement::table)
				.filter(schema::placement::board.eq(self.id))
				.execute(connection)?;
			
			diesel::delete(schema::color::table)
				.filter(schema::color::board.eq(self.id))
				.execute(connection)?;
			
			diesel::delete(schema::board::table)
				.filter(schema::board::id.eq(self.id))
				.execute(connection)?;
			
			Ok(())
		})
	}

	pub fn last_place_time(
		&self,
		user: &User,
		connection: &Connection,
	) -> QueryResult<u32> {
		Ok(schema::placement::table
			.filter(
				schema::placement::board.eq(self.id)
				.and(schema::placement::user_id.eq(user.id.clone()))
			)
			.order((
				schema::placement::timestamp.desc(),
				schema::placement::id.desc(),
			))
			.limit(1)
			.load::<model::Placement>(connection)?
			.pop()
			.map(|placement| placement.timestamp as u32)
			.unwrap_or(0))
	}

	pub fn cooldown(&self) -> u32 {
		30
	}

	pub fn try_place(
		&mut self,
		user: &User,
		position: u64,
		color: u8,
		connection: &Connection,
	) -> Result<model::Placement, PlaceError> {
		// TODO: I hate most things about how this is written. Redo it and/or move stuff.

		let (sector_index, sector_offset) = self.info.shape
			.to_local(position as usize)
			.ok_or(PlaceError::OutOfBounds)?;

		self.info.palette.contains_key(&(color as u32))
			.then(|| ())
			.ok_or(PlaceError::InvalidColor)?;

		let mut sector = self.sectors.write_sector(sector_index, connection)
			.expect("Failed to load sector");

		match FromPrimitive::from_u8(sector.mask[sector_offset]) {
			Some(MaskValue::Place) => Ok(()),
			Some(MaskValue::NoPlace) => Err(PlaceError::Unplacable),
			// NOTE: there exists an old implementation in the version
			// control history. It's messy and would need to load adjacent
			// sectors now so I'm dropping it for now.
			Some(MaskValue::Adjacent) => unimplemented!(),
			None => Err(PlaceError::UnknownMaskValue),
		}?;

		(sector.colors[sector_offset] != color)
			.then(|| ())
			.ok_or(PlaceError::NoOp)?;

		let unix_time = SystemTime::now()
			.duration_since(UNIX_EPOCH).unwrap()
			.as_secs();
		let timestamp = unix_time
			.saturating_sub(self.info.created_at)
			.max(1) as u32;

		((timestamp - self.last_place_time(user, connection).unwrap()) > self.cooldown())
			.then(|| ())
			.ok_or(PlaceError::Cooldown)?;

		let new_placement = diesel::insert_into(schema::placement::table)
			.values(model::NewPlacement {
				board: self.id,
				position: position as i64,
				color: color as i16,
				timestamp: timestamp as i32,
				user_id: user.id.clone(),
			})
			.get_result::<model::Placement>(connection)
			.expect("failed to insert placement");

		sector.colors[sector_offset] = color;
		let timestamp_slice = &mut sector.timestamps[
			(sector_offset * 4)..((sector_offset + 1) * 4)
		];
		timestamp_slice.as_mut().put_u32_le(timestamp as u32);

		Ok(new_placement)
	}

	pub fn list_placements(
		&self,
		timestamp: u32,
		id: usize,
		limit: usize,
		reverse: bool,
		connection: &Connection
	) -> QueryResult<Vec<model::Placement>> {
		// TODO: Reduce duplication.
		// This stems from le and ge having different types, polluting the entire 
		// expression. I suppose the original also had duplication in the sql query,
		// but I guess I was more okay with that?
		if reverse {
			schema::placement::table
				.filter(
					schema::placement::board.eq(self.id)
					.and(
						(schema::placement::timestamp, schema::placement::id)
							.into_sql::<Record<_>>()
							.le((timestamp as i32, id as i64))
					)
				)
				.order((schema::placement::timestamp, schema::placement::id))
				.limit(limit as i64)
				.load::<model::Placement>(connection)
		} else {
			schema::placement::table
				.filter(
					schema::placement::board.eq(self.id)
					.and(
						(schema::placement::timestamp, schema::placement::id)
							.into_sql::<Record<_>>()
							.ge((timestamp as i32, id as i64))
					)
				)
				.order((schema::placement::timestamp, schema::placement::id))
				.limit(limit as i64)
				.load::<model::Placement>(connection)
		}
	}

	pub fn lookup(
		&self,
		position: u64,
		connection: &Connection
	) -> QueryResult<Option<model::Placement>> {
		Ok(schema::placement::table
			.filter(
				schema::placement::board.eq(self.id as i32)
				.and(schema::placement::position.eq(position as i64))
			)
			.order((
				schema::placement::timestamp.desc(),
				schema::placement::id.desc(),
			))
			.limit(1)
			.load::<model::Placement>(connection)?
			.pop())
	}

	pub fn load(
		board: model::Board,
		connection: &Connection,
	) -> QueryResult<Self> {
		let id = board.id;

		let palette = model::Color::belonging_to(&board)
			.load::<model::Color>(connection)?
			.into_iter()
			.map(|color| (color.index as u32, Color::from(color)))
			.collect::<Palette>();

		let info = BoardInfo {
			name: board.name.clone(),
			created_at: board.created_at as u64,
			shape: serde_json::from_value(board.shape).unwrap(),
			palette,
		};

		let sectors = SectorCache::new(
			board.id,
			info.shape.sector_count(),
			info.shape.sector_size(),
		);

		Ok(Board { id, info, sectors })
	}
}

impl From<&Board> for Uri {
	fn from(board: &Board) -> Self {
		format!("/boards/{}", board.id).parse::<Uri>().unwrap()
	}
}

impl<'l> From<&'l Board> for Reference<'l, BoardInfo> {
	fn from(board: &'l Board) -> Self {
		Self {
			uri: board.into(),
			view: &board.info,
		}
	}
}
