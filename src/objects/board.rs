use serde::{Serialize, Deserialize};
use actix_web::web::{BytesMut, Buf, BufMut};
use std::time::{SystemTime, UNIX_EPOCH};
use http::Uri;
use num_derive::FromPrimitive;    
use num_traits::FromPrimitive;
use diesel::types::Record;
use diesel::prelude::*;
use diesel::Connection as DConnection;

use crate::objects::{Color, Reference, Palette, User};
use crate::database::{Connection, model, schema};

// TODO: support other shapes
pub type Shape = [[usize; 2]; 1];

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
	pub id: i32,
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
		connection: &Connection
	) -> QueryResult<Self> {
		let [[width, height]] = info.shape;
		let size = width * height;
		let empty_data = vec![0 as u8; size];

		let new_board: model::Board = diesel::insert_into(schema::board::table)
			.values(model::NewBoard {
				name: info.name,
				created_at: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64,
				shape: serde_json::to_string(&info.shape).unwrap(),
				mask: empty_data.clone(),
				initial: empty_data.clone(),
			})
			.get_result(connection)?;

		crate::objects::color::replace_palette(&info.palette, new_board.id, connection)?;

		Self::load(new_board, connection)
	}

	pub fn update_info(
		&mut self, 
		info: BoardInfoPatch, 
		connection: &Connection,
	) -> QueryResult<()> {
		assert!(info.name.is_some() || info.palette.is_some() || info.shape.is_some());

		connection.transaction(|| {
			if let Some(name) = &info.name {
				diesel::update(schema::board::table)
					.set(schema::board::name.eq(name))
					.filter(schema::board::id.eq(self.id))
					.execute(connection)?;
			}

			if let Some(palette) = &info.palette {
				crate::objects::color::replace_palette(palette, self.id, connection)?;
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

				diesel::update(schema::board::table)
					.set((
						schema::board::shape.eq(serde_json::to_string(shape).unwrap()),
						schema::board::mask.eq(&mask_data[..]),
						schema::board::initial.eq(&initial_data[..]),
					))
					.filter(schema::board::id.eq(self.id))
					.execute(connection)?;

				colors = Some(colors_data);
				timestamps = Some(timestamps_data);
				mask = Some(mask_data);
				initial = Some(initial_data);
			}

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
		})
	}

	pub fn delete(self, connection: &Connection) -> QueryResult<()> {
		connection.transaction(|| {
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
		// TODO: I hate everything about how this is written. Redo it and/oir move stuff.

		match FromPrimitive::from_u8(self.data.mask[position as usize]) {
			Some(MaskValue::Place) => Ok(()),
			Some(MaskValue::NoPlace) => Err(PlaceError::Unplacable),
			Some(MaskValue::Adjacent) => {
				[1, -1, self.info.shape[0][0] as isize, -(self.info.shape[0][0] as isize)]
					.iter()
					.map(|offset| {
						let checked = if offset.is_negative() {
							position.checked_sub(offset.wrapping_abs() as u64)
						} else {
							position.checked_add(*offset as u64)
						};

						checked.and_then(|position| {
							if position < self.data.colors.len() as u64 {
								Some(position)
							} else {
								None
							}
						})
					})
					.flatten()
					.find(|position| {
						let position = (*position as usize) * 4;
						(&self.data.timestamps[position..position + 4])
							.get_u32_le() > 0
					})
					.map(|_| ())
					.ok_or(PlaceError::Unplacable)
			},
			None => Err(PlaceError::UnknownMaskValue),
		}?;

		self.info.palette.contains_key(&(color as u32))
			.then(|| ())
			.ok_or(PlaceError::InvalidColor)?;

		(self.data.colors[position as usize] != color)
			.then(|| ())
			.ok_or(PlaceError::NoOp)?;

		let unix_time = SystemTime::now()
			.duration_since(UNIX_EPOCH).unwrap()
			.as_secs();
		let timestamp = unix_time.saturating_sub(self.info.created_at) as u32;

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
			.expect("place");

		self.do_placement(&new_placement);
		Ok(new_placement)
	}

	fn do_placement(&mut self, placement: &model::Placement) {
		let position = placement.position as usize;
		let range = position..position + 1;
		let range_u32 = position * 4..(position + 1) * 4;

		let color_slice = &mut self.data.colors[range];
		color_slice.as_mut().put_u8(placement.color as u8);
		
		let timestamp_slice = &mut self.data.timestamps[range_u32];
		timestamp_slice.as_mut().put_u32_le(placement.timestamp as u32);
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
		x: usize,
		y: usize,
		connection: &Connection
	) -> QueryResult<Option<model::Placement>> {
		// TODO: convert from arbitrary shapes
		let position = x + y * self.info.shape[0][0];

		Ok(schema::placement::table
			.filter(
				schema::placement::board.eq(self.id as i32)
				.and(schema::placement::position.eq(position as i64))
			)
			.order((
				schema::placement::timestamp.desc(),
				schema::placement::id.desc(),
			))
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
			shape: serde_json::from_str(&board.shape).unwrap(),
			palette,
		};
		
		let [width, height] = info.shape[0];
		let size = width * height;
		assert_eq!(size, board.mask.len());
		assert_eq!(size, board.initial.len());
		let mut color_data = BytesMut::from(&board.initial[..]);
		let mut timestamp_data = BytesMut::from(&vec![0; size * 4][..]);

		let placements = model::Placement::belonging_to(&board)
			.load::<model::Placement>(connection)?;

		for placement in placements {
			let index = placement.position as usize;
			color_data[index] = placement.color as u8;
			let timestamp_slice = &mut timestamp_data[index * 4..index * 4 + 4];
			timestamp_slice.as_mut().put_u32_le(placement.timestamp as u32);
		};
		let data = BoardData {
			colors: color_data,
			timestamps: timestamp_data,
			mask: BytesMut::from(&board.mask[..]),
			initial: BytesMut::from(&board.initial[..]),
		};
	
		Ok(Board { id, info, data })
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
