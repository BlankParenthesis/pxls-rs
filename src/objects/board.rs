use serde::{Serialize, Deserialize};
use actix_web::web::{Bytes, BytesMut, BufMut};
use std::sync::RwLock;
use std::time::{SystemTime, UNIX_EPOCH};
use std::collections::{VecDeque, HashMap};
use std::convert::TryFrom;
use r2d2_sqlite::rusqlite::Result;
use rusqlite::params;
use http::Uri;

use crate::objects::{Color, Placement, Reference, Palette};
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
	pub mask: Bytes,
	pub initial: Bytes,
}

pub struct Board {
	pub id: usize,
	pub info: BoardInfo,
	pub data: BoardData,
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

		let mut mask = None;
		let mut initial = None;

		if let Some(shape) = &info.shape {
			let [[width, height]] = shape;
			let size = width * height;

			mask = Some({
				let mut mask = BytesMut::from(&self.data.mask[..]);
				mask.resize(size, 0);
				mask.freeze()
			});
			initial = Some({
				let mut initial = BytesMut::from(&self.data.initial[..]);
				initial.resize(size, 0);
				initial.freeze()
			});

			transaction.execute(
				"UPDATE `board` SET `shape` = ?2, `mask` = ?3, `initial` = ?4 WHERE `id` = ?1",
				params![
					self.id,
					serde_json::to_string(shape).unwrap(),
					&mask.as_ref().unwrap()[..],
					&initial.as_ref().unwrap()[..],
				],
			)?;
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
			self.data.mask = mask.unwrap();
			self.data.initial = initial.unwrap();
		}

		Ok(())
	}

	pub fn put_color(&mut self, index: usize, color: u8) {
		// NOTE: this creates a timestamp for when the request was made.
		// It could be put before the lock so that the timestamp is for when the
		// request is actually honoured.
		let timestamp = SystemTime::now()
			.duration_since(UNIX_EPOCH).unwrap()
			.as_secs();
		let delta = timestamp.saturating_sub(self.info.created_at);

		let color_slice = &mut self.data.colors[index..index + 1];
		color_slice.as_mut().put_u8(color);
		
		let timestamp_slice = &mut self.data.timestamps[index..index + 4];
		timestamp_slice.as_mut().put_u32_le(delta as u32);
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
						position: placement.get(0)?,
						color: placement.get(1)?,
						modified: placement.get(2)?,
					}))?
					.collect::<Result<_>>()?;
				for placement in placements {
					let index = placement.position;
					color_data[index] = placement.color;
					let timestamp_slice = &mut timestamp_data[index * 4..index * 4 + 4];
					timestamp_slice.as_mut().put_u32_le(placement.modified);
				};
			
				let data = BoardData {
					colors: color_data,
					timestamps: timestamp_data,
					mask: Bytes::from(board.mask),
					initial: Bytes::from(board.initial),
				};
			
				Ok(Board { id, info, data })
			})?
			.collect::<Result<VecDeque<_>>>()?
			.pop_front())
	}
}