use std::{convert::*, io::*};

use diesel::{Connection as _, QueryResult};
use parking_lot::*;

use crate::{
	database::Connection,
	objects::{BoardSector, SectorBuffer},
};

pub struct SectorCache {
	// TODO: evict based on size
	board_id: i32,
	sector_size: usize,
	sectors: Vec<RwLock<Option<BoardSector>>>,
}

impl SectorCache {
	pub fn new(
		board_id: i32,
		sector_count: usize,
		sector_size: usize,
	) -> Self {
		let mut sectors = Vec::new();
		sectors.resize_with(sector_count, Default::default);

		Self {
			board_id,
			sector_size,
			sectors,
		}
	}

	fn fill_sector(
		&self,
		sector_index: usize,
		connection: &Connection,
	) -> QueryResult<RwLockWriteGuard<Option<BoardSector>>> {
		let mut option = self
			.sectors
			.get(sector_index)
			.unwrap()
			.write();

		let load = BoardSector::load(
			self.board_id,
			sector_index as i32,
			connection,
		)?;

		let sector = match load {
			Some(sector) => sector,
			None => {
				BoardSector::new(
					self.board_id,
					sector_index as i32,
					self.sector_size,
					connection,
				)?
			},
		};

		option.replace(sector);

		Ok(option)
	}

	pub fn evict_sector(
		&self,
		sector_index: usize,
	) -> Option<BoardSector> {
		let mut option = self
			.sectors
			.get(sector_index)
			.unwrap()
			.write();

		option.take()
	}

	pub fn read_sector(
		&self,
		sector_index: usize,
		connection: &Connection,
	) -> Option<MappedRwLockReadGuard<BoardSector>> {
		if let Some(lock) = self.sectors.get(sector_index) {
			let option = lock.read();
			if option.is_some() {
				Some(RwLockReadGuard::map(option, |o| o.as_ref().unwrap()))
			} else {
				drop(option);

				let sector = self
					.fill_sector(sector_index, connection)
					.unwrap();

				Some(RwLockReadGuard::map(
					RwLockWriteGuard::downgrade(sector),
					|o| o.as_ref().unwrap(),
				))
			}
		} else {
			None
		}
	}

	pub fn write_sector(
		&self,
		sector_index: usize,
		connection: &Connection,
	) -> Option<MappedRwLockWriteGuard<BoardSector>> {
		if let Some(lock) = self.sectors.get(sector_index) {
			let option = lock.write();
			if option.is_some() {
				Some(RwLockWriteGuard::map(option, |o| o.as_mut().unwrap()))
			} else {
				drop(option);

				let sector = self
					.fill_sector(sector_index, connection)
					.unwrap();

				Some(RwLockWriteGuard::map(sector, |o| o.as_mut().unwrap()))
			}
		} else {
			None
		}
	}

	pub fn access<'l>(
		&'l self,
		buffer: SectorBuffer,
		connection: &'l Connection,
	) -> SectorCacheAccess<'l> {
		SectorCacheAccess {
			cursor: 0,
			buffer,
			sectors: self,
			connection,
		}
	}
}

pub trait Len {
	fn len(&self) -> usize;

	fn is_empty(&self) -> bool {
		self.len() == 0
	}
}

pub struct SectorCacheAccess<'l> {
	cursor: usize,
	buffer: SectorBuffer,
	sectors: &'l SectorCache,
	connection: &'l Connection,
}

impl<'l> SectorCacheAccess<'l> {
	fn sector_size(&self) -> usize {
		self.sectors.sector_size
			* match self.buffer {
				SectorBuffer::Colors => 1,
				SectorBuffer::Timestamps => 4,
				SectorBuffer::Initial => 1,
				SectorBuffer::Mask => 1,
			}
	}
}

impl<'l> Len for SectorCacheAccess<'l> {
	fn len(&self) -> usize {
		self.sectors.sectors.len() * self.sector_size()
	}
}

impl<'l> Seek for SectorCacheAccess<'l> {
	fn seek(
		&mut self,
		seek: SeekFrom,
	) -> std::result::Result<u64, std::io::Error> {
		let new_cursor = match seek {
			SeekFrom::Current(value) => {
				i64::try_from(self.cursor)
					.map(|cursor| {
						cursor
							.checked_add(value)
							.expect("overflow/underflow on seek")
					})
					.and_then(u64::try_from)
			},
			SeekFrom::End(value) => {
				i64::try_from(self.len())
					.map(|end| {
						end.checked_sub(value)
							.expect("overflow/underflow on seek")
					})
					.and_then(u64::try_from)
			},
			SeekFrom::Start(value) => Ok(value),
		};

		new_cursor
			.and_then(usize::try_from)
			.map(|new_cursor| self.cursor = new_cursor)
			.and_then(|_| u64::try_from(self.cursor))
			.map_err(|e| std::io::Error::new(ErrorKind::Other, e))
	}
}

impl<'l> Read for SectorCacheAccess<'l> {
	fn read(
		&mut self,
		mut output: &mut [u8],
	) -> std::result::Result<usize, std::io::Error> {
		let mut written = 0;
		let total_size = self.len();
		let sector_size = self.sector_size();

		while !output.is_empty() && (0..total_size).contains(&self.cursor) {
			let sector_index = self.cursor / sector_size;

			let offset = self.cursor % sector_size;

			let sector = self
				.sectors
				.read_sector(sector_index, self.connection)
				.unwrap();

			let mut buf = &match self.buffer {
				SectorBuffer::Colors => &sector.colors,
				SectorBuffer::Timestamps => &sector.timestamps,
				SectorBuffer::Initial => &sector.initial,
				SectorBuffer::Mask => &sector.mask,
			}[offset..];

			let write_len = buf.read(output)?;

			if write_len == 0 {
				break;
			}

			output = &mut output[write_len..];
			written += write_len;
			self.cursor += write_len;
		}

		Ok(written)
	}
}

impl<'l> Write for SectorCacheAccess<'l> {
	fn write(
		&mut self,
		mut input: &[u8],
	) -> std::result::Result<usize, std::io::Error> {
		let total_size = self.len();
		let sector_size = self.sector_size();

		let input = &mut input;

		Ok(self
			.connection
			.transaction::<usize, diesel::result::Error, _>(|| {
				let mut written = 0;

				while !input.is_empty() && (0..total_size).contains(&self.cursor) {
					let sector_index = self.cursor / sector_size;

					let offset = self.cursor % sector_size;

					let mut sector = self
						.sectors
						.write_sector(sector_index, self.connection)
						.unwrap();

					let buf = &mut match self.buffer {
						SectorBuffer::Colors => &mut sector.colors,
						SectorBuffer::Timestamps => &mut sector.timestamps,
						SectorBuffer::Initial => &mut sector.initial,
						SectorBuffer::Mask => &mut sector.mask,
					}[offset..];

					let write_len: usize = input.read(buf).unwrap();

					if write_len == 0 {
						break;
					}

					written = write_len;
					self.cursor += write_len;

					sector.save(self.connection, Some(&self.buffer))?;

					if self.buffer == SectorBuffer::Initial {
						drop(sector);
						self.sectors.evict_sector(sector_index);
					}
				}

				Ok(written)
			})
			.unwrap())
	}

	fn flush(&mut self) -> std::result::Result<(), std::io::Error> {
		Ok(())
	}
}
