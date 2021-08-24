use serde::Serialize;
use actix_web::web::{Bytes, BytesMut, BufMut};
use std::sync::RwLock;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::objects::color::Color;

#[derive(Serialize, Debug)]
pub struct BoardInfo {
	pub name: String,
	pub created_at: u64,
	pub shape: [[usize; 2]; 1], // TODO: support other shapes
	pub palette: Vec<Color>,
}

pub struct BoardData {
	pub colors: BytesMut,
	pub timestamps: BytesMut,
	pub mask: Bytes,
	pub initial: Bytes,
}

pub struct Board {
	pub info: BoardInfo,
	pub data: RwLock<BoardData>,
}

impl Board {
	pub fn new(
		name: String, 
		created_at: u64, 
		shape: [[usize; 2]; 1],
		palette: Vec<Color>,
	) -> Self {
		let [[width, height]] = shape;
		let size = width * height;

		Board {
			info: BoardInfo {
				name,
				created_at,
				shape,
				palette,
			},
			data: RwLock::new(BoardData {
				colors: BytesMut::from(&vec![0; size][..]),
				timestamps: BytesMut::from(&vec![0; size * 4][..]),
				mask: BytesMut::from(&vec![0; size][..]).freeze(),
				initial: BytesMut::from(&vec![0; size][..]).freeze(),
			})
		}
	}
	
	pub fn put_color(&mut self, index: usize, color: u8) {
		// NOTE: this creates a timestamp for when the request was made.
		// It could be put before the lock so that the timestamp is for when the
		// request is actually honoured.
		let timestamp = SystemTime::now()
			.duration_since(UNIX_EPOCH).unwrap()
			.as_secs();
		let delta = timestamp.saturating_sub(self.info.created_at);

		let data = self.data.get_mut().unwrap();

		let color_slice = &mut data.colors[index..index + 1];
		color_slice.as_mut().put_u8(color);
		
		let timestamp_slice = &mut data.timestamps[index..index + 4];
		timestamp_slice.as_mut().put_u32_le(delta as u32);
	}
}