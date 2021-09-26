use serde::{Serialize, Deserialize};
use actix_web::web::{Bytes, BytesMut, Buf, BufMut};
use std::time::{SystemTime, UNIX_EPOCH};
use std::collections::{VecDeque, HashMap};
use std::convert::TryFrom;
use r2d2_sqlite::rusqlite::Result;
use rusqlite::params;
use http::Uri;
use num_derive::FromPrimitive;    
use num_traits::FromPrimitive;

use crate::objects::{Color, Placement, Reference, Palette, User};
use crate::database::queries::{Connection, FromDatabase};

// TODO: support other shapes
type Shape = [[usize; 2]; 1];

#[derive(Serialize, Debug)]
pub struct BoardInfo {
	pub name: String,
	pub created_at: u64,
	pub shape: Shape,
	pub palette: Palette,
}

#[derive(Deserialize, Debug)]
pub struct BoardInfoPost {
	pub name: String,
	pub shape: Shape,
	pub palette: Palette,
}


#[derive(Deserialize, Debug)]
pub struct BoardInfoPatch {
	pub name: Option<String>,
	pub shape: Option<Shape>,
	pub palette: Option<Palette>,
}

pub struct BoardData {
	pub colors: BytesMut,
	pub timestamps: BytesMut,
	pub mask: BytesMut,
	pub initial: BytesMut,
}

pub struct Board {
	pub id: usize,
	pub info: BoardInfo,
	pub data: BoardData,
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
		}
	}
}

impl Board {
	pub fn create(
		info: BoardInfoPost, 
		connection: &mut Connection
	) -> Result<Self> {
		let [[width, height]] = info.shape;
		let size = width * height;
		let empty_data = vec![0 as u8; size];

		connection.execute("INSERT INTO `board` VALUES(null, ?1, ?2, ?3, ?4, ?5)", params![
			info.name,
			SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs(),
			serde_json::to_string(&info.shape).unwrap(),
			empty_data,
			empty_data,
		])?;

		let id = connection.last_insert_rowid() as usize;

		crate::objects::color::save_palette(&info.palette, id, connection)?;

		Self::load(id, connection).map(|option| option.unwrap())
	}

	pub fn update_info(
		&mut self, 
		info: BoardInfoPatch, 
		connection: &mut Connection,
	) -> Result<()> {
		assert!(info.name.is_some() || info.palette.is_some() || info.shape.is_some());

		let transaction = connection.transaction()?;

		if let Some(name) = &info.name {
			transaction.execute(
				"UPDATE `board` SET `name` = ?2 WHERE `id` = ?1",
				params![self.id, name],
			)?;
		}

		if let Some(palette) = &info.palette {
			transaction.execute(
				"DELETE FROM `color` WHERE `board` = ?1",
				params![self.id],
			)?;

			crate::objects::color::save_palette_transaction(palette, self.id, &transaction)?;
		}

		let mut colors = None;
		let mut timestamps = None;
		let mut mask = None;
		let mut initial = None;

		if let Some(shape) = &info.shape {
			let [[width, height]] = shape;
			let size = width * height;

			let mut colors_data = BytesMut::from(&self.data.colors[..]);
			colors_data.resize(size, 0);

			let mut timestamps_data = BytesMut::from(&self.data.timestamps[..]);
			timestamps_data.resize(size * 4, 0);

			let mut mask_data = BytesMut::from(&self.data.mask[..]);
			mask_data.resize(size, 2);

			let mut initial_data = BytesMut::from(&self.data.initial[..]);
			initial_data.resize(size, 0);

			transaction.execute(
				"UPDATE `board` SET `shape` = ?2, `mask` = ?3, `initial` = ?4 WHERE `id` = ?1",
				params![
					self.id,
					serde_json::to_string(shape).unwrap(),
					&mask_data[..],
					&initial_data[..],
				],
			)?;

			colors = Some(colors_data);
			timestamps = Some(timestamps_data);
			mask = Some(mask_data);
			initial = Some(initial_data);
		}

		transaction.commit()?;

		if let Some(name) = info.name {
			self.info.name = name;
		}

		if let Some(palette) = info.palette {
			self.info.palette = palette;
		}

		if let Some(shape) = info.shape {
			self.info.shape = shape;
			self.data.colors = colors.unwrap();
			self.data.timestamps = timestamps.unwrap();
			self.data.mask = mask.unwrap();
			self.data.initial = initial.unwrap();
		}

		Ok(())
	}

	pub fn delete(self, connection: &mut Connection) -> Result<()> {
		let transaction = connection.transaction()?;

		transaction.execute("DELETE FROM `placement` WHERE `board` = ?1", [self.id])?;
		transaction.execute("DELETE FROM `color` WHERE `board` = ?1", [self.id])?;
		transaction.execute("DELETE FROM `board` WHERE `id` = ?1", [self.id])?;

		transaction.commit()?;

		Ok(())
	}

	pub fn try_place(
		&mut self,
		user: &User,
		position: usize,
		color: u8,
		connection: &mut Connection,
	) -> std::result::Result<Placement, PlaceError> {
		// TODO: I hate everything about how this is written. Redo it and/oir move stuff.

		match FromPrimitive::from_u8(self.data.mask[position]) {
			Some(MaskValue::Place) => Ok(()),
			Some(MaskValue::NoPlace) => Err(PlaceError::Unplacable),
			Some(MaskValue::Adjacent) => {
				[1, -1, self.info.shape[0][0] as isize, -(self.info.shape[0][0] as isize)]
					.iter()
					.map(|offset| {
						let checked = if offset.is_negative() {
							position.checked_sub(offset.wrapping_abs() as usize)
						} else {
							position.checked_add(*offset as usize)
						};

						checked.and_then(|position| {
							if position < self.data.colors.len() {
								Some(position)
							} else {
								None
							}
						})
					})
					.flatten()
					.find(|position| {
						let position = position * 4;
						(&self.data.timestamps[position..position + 4])
							.get_u32_le() > 0
					})
					.map(|_| ())
					.ok_or(PlaceError::Unplacable)
			},
			None => Err(PlaceError::UnknownMaskValue),
		}?;

		self.info.palette.contains_key(&(color as usize))
			.then(|| ())
			.ok_or(PlaceError::InvalidColor)?;

		(self.data.colors[position] != color)
			.then(|| ())
			.ok_or(PlaceError::NoOp)?;

		let unix_time = SystemTime::now()
			.duration_since(UNIX_EPOCH).unwrap()
			.as_secs();
		let timestamp = unix_time.saturating_sub(self.info.created_at) as u32;

		//((unix_time - user.last_place_time) > cooldown)
		//	.then(|| ())
		//	.ok_or(PlaceError::Cooldown)?;

		connection.execute(
			include_str!("../database/sql/insert_placement.sql"),
			params![self.id, position, color, timestamp, user.id]
		).expect("insert");

		let id = connection.last_insert_rowid() as usize;

		let placement = Placement {
			id,
			position, 
			color, 
			timestamp,
		};

		self.do_placement(&placement);
		Ok(placement)
	}

	fn do_placement(&mut self, placement: &Placement) {
		let position = placement.position;
		let range = position..position + 1;
		let range_u32 = position * 4..(position + 1) * 4;

		let color_slice = &mut self.data.colors[range];
		color_slice.as_mut().put_u8(placement.color);
		
		let timestamp_slice = &mut self.data.timestamps[range_u32];
		timestamp_slice.as_mut().put_u32_le(placement.timestamp);
	}

	pub fn list_placements(
		&self,
		timestamp: u32,
		id: usize,
		limit: usize,
		reverse: bool,
		connection: &mut Connection
	) -> Result<Vec<Placement>> {
		let query = if reverse {
			include_str!("../database/sql/placement_page_backward.sql")
		} else {
			include_str!("../database/sql/placement_page_forward.sql")
		};

		connection.prepare(query)?
			.query_map(params![self.id, timestamp, id, limit], |placement| {
				Ok(Placement {
					id: placement.get(0)?,
					position: placement.get(1)?,
					color: placement.get(2)?,
					timestamp: placement.get(3)?,
				})
			})?
			.collect()

	}

	pub fn lookup(
		&self,
		x: usize,
		y: usize,
		connection: &mut Connection
	) -> Result<Option<Placement>> {
		// TODO: convert from arbitrary shapes
		let position = x + y * self.info.shape[0][0];

		Ok(connection.prepare(include_str!("../database/sql/lookup.sql"))?
			.query_map(params![self.id, position], |placement| {
				Ok(Placement {
					id: placement.get(0)?,
					position: placement.get(1)?,
					color: placement.get(2)?,
					timestamp: placement.get(3)?,
				})
			})?
			.collect::<Result<Vec<_>, _>>()?
			.pop())
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

struct BoardRow {
	id: usize,
	name: String,
	created_at: u64,
	shape: [[usize; 2]; 1],
	mask: Vec<u8>,
	initial: Vec<u8>,
}

impl TryFrom<&rusqlite::Row<'_>> for BoardRow {
	type Error = rusqlite::Error;

	fn try_from(row: &rusqlite::Row) -> Result<Self> {
		let shape_json_string = (row.get(3) as Result<String>)?;

		Ok(Self {
			id: row.get(0)?,
			name: row.get(1)?,
			created_at: row.get(2)?,
			// TODO: propagate error rather than unwrapping
			shape: serde_json::de::from_str(&shape_json_string).unwrap(),
			mask: row.get(4)?,
			initial: row.get(5)?,
		})
	}
}

impl FromDatabase for Board {
	fn load(id: usize, connection: &Connection) -> Result<Option<Self>> {
		Ok(connection.prepare("SELECT `id`, `name`, `created_at`, `shape`, `mask`, `initial` FROM `board` WHERE `id` = ?1")?
			.query_map([id], |row| {
				let board = BoardRow::try_from(row)?;
				
				let palette: HashMap<usize, Color> = connection
					.prepare("SELECT `index`, `name`, `value` FROM `color` WHERE `board` = ?1")?
					.query_map([id], |color| Ok((
						color.get(0)?,
						Color {
							name: color.get(1)?,
							value: color.get(2)?,
						},
					)))?
					.collect::<Result<_>>()?;

			
				let info = BoardInfo {
					name: board.name,
					created_at: board.created_at,
					shape: board.shape,
					palette,
				};
			
				let [width, height] = info.shape[0];
				let size = width * height;
				assert_eq!(size, board.mask.len());
				assert_eq!(size, board.initial.len());
				let mut color_data = BytesMut::from(&board.initial[..]);
				let mut timestamp_data = BytesMut::from(&vec![0; size * 4][..]);
			
				let placements: Vec<Placement> = connection
					.prepare(include_str!("../database/sql/current_placements.sql"))?
					.query_map([id], |placement| Ok(Placement {
						id: placement.get(0)?,
						position: placement.get(1)?,
						color: placement.get(2)?,
						timestamp: placement.get(3)?,
					}))?
					.collect::<Result<_>>()?;
				for placement in placements {
					let index = placement.position;
					color_data[index] = placement.color;
					let timestamp_slice = &mut timestamp_data[index * 4..index * 4 + 4];
					timestamp_slice.as_mut().put_u32_le(placement.timestamp);
				};
			
				let data = BoardData {
					colors: color_data,
					timestamps: timestamp_data,
					mask: BytesMut::from(&board.mask[..]),
					initial: BytesMut::from(&board.initial[..]),
				};
			
				Ok(Board { id, info, data })
			})?
			.collect::<Result<VecDeque<_>>>()?
			.pop_front())
	}
}