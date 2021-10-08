use serde::{Serialize, Deserialize};
use actix::prelude::*;
use actix_web::web::BufMut;
use std::time::{SystemTime, UNIX_EPOCH};
use std::io::{Write, Seek, SeekFrom};
use std::sync::Arc;
use std::collections::HashSet;
use std::convert::TryFrom;
use http::Uri;
use num_derive::FromPrimitive;
use num_traits::FromPrimitive;
use diesel::types::Record;
use diesel::prelude::*;
use diesel::Connection as DConnection;
use actix_web::http::{HeaderName, HeaderValue};

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
	SectorCacheAccess,
	UserCount,
};
use crate::socket::server::{BoardServer, RunEvent};
use crate::socket::socket::{BoardSocket, Extension};
use crate::socket::event::{Event, BoardInfo as EventBoardInfo, BoardData, Change};

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
	sectors: SectorCache,
	server: Arc<Addr<BoardServer>>,
}

pub struct CooldownInfo {
	pixels_available: usize,
	next_available: Option<u32>,
}

impl CooldownInfo {
	pub fn into_headers(self) -> Vec<(HeaderName, HeaderValue)> {
		let mut headers = vec![
			(
				HeaderName::from_static("pxls-pixels-available"),
				self.pixels_available.into()
			),
		];

		if let Some(next_available) = self.next_available {
			headers.push(
				(
					HeaderName::from_static("pxls-next-available"),
					next_available.into()
				)
			);
		}

		headers
	}
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

	pub fn read<'l>(
		&'l self,
		buffer: SectorBuffer,
		connection: &'l Connection,
	) -> SectorCacheAccess<'l> {
		self.sectors.access(buffer, connection)
	}

	// TODO: proper error type
	pub fn try_patch_initial(
		&self,
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

		let event = Event::BoardUpdate {
			info: None,
			data: Some(BoardData {
				colors: None,
				timestamps: None,
				initial: Some(vec![Change {
					position: patch.start,
					values: Vec::from(&*patch.data),
				}]),
				mask: None,
			}),
		};

		self.server.do_send(RunEvent { event });

		Ok(())
	}

	pub fn try_patch_mask(
		&self,
		patch: &BinaryPatch,
		connection: &Connection
	) -> Result<(), &'static str> {
		let mut sector_data = self.sectors
			.access(SectorBuffer::Mask, connection);

		sector_data.seek(SeekFrom::Start(patch.start))
			.map_err(|_| "invalid start position")?;

		sector_data.write(&*patch.data)
			.map_err(|_| "write error")?;

		let event = Event::BoardUpdate {
			info: None,
			data: Some(BoardData {
				colors: None,
				timestamps: None,
				initial: None,
				mask: Some(vec![Change {
					position: patch.start,
					values: Vec::from(&*patch.data),
				}]),
			}),
		};

		self.server.do_send(RunEvent { event });

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

		if let Some(ref name) = info.name {
			self.info.name = name.clone();
		}

		if let Some(ref palette) = info.palette {
			self.info.palette = palette.clone();
		}

		if let Some(ref shape) = info.shape {
			self.info.shape = shape.clone();

			self.sectors = SectorCache::new(
				self.id,
				self.info.shape.sector_count(),
				self.info.shape.sector_size(),
			)
		}

		let event = Event::BoardUpdate {
			info: Some(EventBoardInfo {
				name: info.name,
				palette: info.palette,
				shape: info.shape,
			}),
			data: None,
		};

		self.server.do_send(RunEvent { event });

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
		// TODO: proper cooldown
		30
	}

	pub fn try_place(
		&self,
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

		let timestamp = self.current_timestamp();
		let cooldown_info = self.user_cooldown_info(user, connection).unwrap();
		
		if cooldown_info.pixels_available == 0 {
			return Err(PlaceError::Cooldown);
		}

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
		timestamp_slice.as_mut().put_u32_le(timestamp);

		let event = Event::BoardUpdate {
			info: None,
			data: Some(BoardData {
				colors: Some(vec![Change {
					position,
					values: vec![color as u8],
				}]),
				timestamps: Some(vec![Change {
					position,
					values: vec![timestamp],
				}]),
				initial: None,
				mask: None,
			}),
		};

		self.server.do_send(RunEvent { event });

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

		let server = Arc::new(BoardServer::default().start());

		Ok(Board { id, info, sectors, server })
	}

	fn current_timestamp(&self) -> u32 {
		let unix_time = SystemTime::now()
			.duration_since(UNIX_EPOCH).unwrap()
			.as_secs();

		u32::try_from(unix_time
			.saturating_sub(self.info.created_at)
			.max(1))
			.unwrap()
	}

	// TODO: move to a field of info
	const MAX_STACKED: usize = 6;

	// TODO: replace with a better function (or set of functions)
	fn cooldown_info(&self, time_elapsed: u32) -> CooldownInfo {
		let cooldown = self.cooldown();
		let pixels_available = ((time_elapsed / cooldown) as usize)
			.min(Board::MAX_STACKED);
		if pixels_available == Board::MAX_STACKED {
			CooldownInfo {
				pixels_available,
				next_available: None,
			}
		} else {
			let remaining = cooldown - (time_elapsed % cooldown);

			CooldownInfo {
				pixels_available,
				next_available: Some(self.current_timestamp() + remaining),
			}
		}
	}

	pub fn user_cooldown_info(
		&self,
		user: &User,
		connection: &Connection,
	) -> QueryResult<CooldownInfo> {
		let current_time = self.current_timestamp();

		let placements = std::iter::once(0)
			.chain(schema::placement::table
				.select(schema::placement::timestamp)
				.filter(schema::placement::board.eq(self.id)
					.and(schema::placement::user_id.eq(user.id.as_ref())))
				.order((schema::placement::timestamp.desc(), schema::placement::id.desc()))
				.limit(Board::MAX_STACKED as i64)
				.get_results::<i32>(connection)?
				.into_iter()
				.rev())
			.map(|i| i as u32)
			.collect::<Vec<_>>();

		match &*placements {
			// no placements have occurred yet, the identity cooldown for this
			// time is fine.
			&[_zero] => Ok(self.cooldown_info(current_time)),
			// determining cooldown is non-trivial
			placements => {
				let mut info = self.cooldown_info(
					current_time.saturating_sub(*placements.last().unwrap()),
				);

				// If we would already have MAX_STACKED just from waiting, we
				// don't need to check previous data since we can't possibly
				// have more.
				// Similarly, we know we needed to spend a pixel on the most
				// recent placement so we can't have saved more than 
				// `MAX_STACKED - 1` since then.
				// TODO: actually, I think this generalizes and we only have to
				// check the last `Board::MAX_STACKED - current_stacked` pixels.
				if info.pixels_available < (Board::MAX_STACKED - 1) {
					let mut pixels: usize = 0;
			
					// In order to place MAX_STACKED pixels, a user must either:
					// - start with MAX_STACKED already stacked or
					// - wait between each placement enough to gain the pixels.
					// By looking at how many pixels a user would have gained
					// between each placement we can determine a minimum number
					// of pixels, and by assuming they start with MAX_STACKED we
					// can  also infer a maximum.
					// These bounds necessarily converge after looking at
					// MAX_STACKED placements because of the two conditions
					// outlined above.

					// NOTE: an important assumption here is that to stack N
					// pixels it takes the same amount of time from the last
					// placement __regardless__ of what the current stack is.
		

					for pair in placements.windows(2) {
						let delta = pair[1] - pair[0];
		
						let info = self.cooldown_info(delta);
		
						pixels = pixels.max(info.pixels_available)
							.saturating_sub(1);
					}
		
					info.pixels_available = info.pixels_available.max(pixels);
				}
	
				Ok(info)
			},
		}
	}

	pub async fn user_count(
		&self,
		connection: &Connection,
	) -> QueryResult<UserCount> {
		// in seconds
		let idle_timeout = 5 * 60;
		let current_time = self.current_timestamp();
		let threshold_time = i32::try_from(current_time.saturating_sub(idle_timeout)).unwrap();

		#[derive(QueryableByName)]
		struct Count {
			#[sql_type = "diesel::sql_types::Int8"]
			active: i64,
		}

		// TODO: this is possible in diesel's master branch but not available yet
		/*
		let active = schema::placement::table.select(
				diesel::dsl::count_distinct(schema::placement::user_id)
			).filter(schema::placement::board.eq(self.id)
				.and(schema::placement::timestamp.gt(threshold_time)))
			.get_result::<i64>(connection)? as usize;
		*/
		// so instead we have this ugliness 😭:

		let count = diesel::sql_query("
			SElECT COUNT(DISTINCT user_id) AS active
			FROM placement
			WHERE board = $1
			AND timestamp > $2")
			.bind::<diesel::sql_types::Int4, _>(self.id)
			.bind::<diesel::sql_types::Int4, _>(threshold_time)
			.get_result::<Count>(connection)?;
		
		let active = count.active as usize;

		Ok(UserCount { idle_timeout, active })
	}

	pub fn new_socket(&self, extensions: HashSet<Extension>) -> BoardSocket {
		BoardSocket {
			extensions,
			server: self.server.clone()
		}
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
