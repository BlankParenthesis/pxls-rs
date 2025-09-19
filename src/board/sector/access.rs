use std::{convert::*, io::*};
use std::io::Read;

use async_trait::async_trait;
use reqwest::StatusCode;
use warp::reply::Reply;

use crate::database::{BoardsConnection, BoardsDatabaseError};
use crate::{AsyncRead, AsyncWrite, Len};
use super::{SectorBuffer, SectorCache};

#[derive(Debug)]
pub enum IoError {
	Io(std::io::Error),
	Sql(BoardsDatabaseError),
}

impl Reply for IoError {
	fn into_response(self) -> warp::reply::Response {
		StatusCode::INTERNAL_SERVER_ERROR.into_response()
	}
}

impl From<std::io::Error> for IoError {
	fn from(value: std::io::Error) -> Self {
		Self::Io(value)
	}
}

impl From<BoardsDatabaseError> for IoError {
	fn from(value: BoardsDatabaseError) -> Self {
		Self::Sql(value)
	}
}

pub struct SectorAccessor<'l> {
	cursor: usize,
	buffer: SectorBuffer,
	sectors: &'l SectorCache,
	connection: &'l BoardsConnection,
}

impl<'l> SectorAccessor<'l> {
	pub fn new(
		sectors: &'l SectorCache,
		buffer: SectorBuffer,
		connection: &'l BoardsConnection,
	) -> Self {
		SectorAccessor {
			cursor: 0,
			buffer,
			sectors,
			connection,
		}
	}

	fn sector_size(&self) -> usize {
		self.sectors.sector_size() * self.buffer.size()
	}
}

impl<'l> Len for SectorAccessor<'l> {
	fn len(&self) -> usize {
		self.sectors.total_sectors() * self.sector_size()
	}
}

impl<'l> Seek for SectorAccessor<'l> {
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
							.expect("overflow/underflow on seek") // TODO: bad expect?
					})
					.and_then(u64::try_from)
			},
			SeekFrom::End(value) => {
				i64::try_from(self.len())
					.map(|end| {
						end.checked_sub(value)
							.expect("overflow/underflow on seek") // TODO: bad expect?
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

#[async_trait]
impl<'l> AsyncRead for SectorAccessor<'l> {
	type Error = IoError;

	async fn read(
		&mut self,
		mut output: &mut [u8],
	) -> std::result::Result<usize, Self::Error> {
		let mut written = 0;
		let total_size = self.len();
		let sector_size = self.sector_size();

		while !output.is_empty() && (0..total_size).contains(&self.cursor) {
			let sector_index = self.cursor / sector_size;

			let offset = self.cursor % sector_size;

			let sector = self.sectors
				.get_sector(sector_index, self.connection).await?
				.unwrap();

			let mut buf = &match self.buffer {
				SectorBuffer::Colors => &sector.colors.data,
				SectorBuffer::Timestamps => &sector.timestamps.data,
				SectorBuffer::Initial => &sector.initial.data,
				SectorBuffer::Mask => &sector.mask.data,
				SectorBuffer::Density => &sector.density.data,
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
	
#[async_trait]
impl<'l> AsyncWrite for SectorAccessor<'l> {
	type Error = IoError;

	async fn write(
		&mut self,
		mut input: &[u8],
	) -> std::result::Result<usize, Self::Error> {
		let total_size = self.len();
		let sector_size = self.sector_size();

		let mut written = 0;

		let transaction = self.connection.begin().await?;

		while !input.is_empty() && (0..total_size).contains(&self.cursor) {
			let sector_index = self.cursor / sector_size;

			let offset = self.cursor % sector_size;

			let mut sector = self.sectors
				.get_sector_mut(sector_index, &transaction)
				.await?
				.expect("Missing sector");

			let buf = &mut match self.buffer {
				SectorBuffer::Colors => &mut sector.colors.data,
				SectorBuffer::Timestamps => &mut sector.timestamps.data,
				SectorBuffer::Initial => &mut sector.initial.data,
				SectorBuffer::Mask => &mut sector.mask.data,
				SectorBuffer::Density => &mut sector.density.data,
			}[offset..];

			let write_len: usize = input.read(buf)?;

			if write_len == 0 {
				break;
			}

			written += write_len;
			self.cursor += write_len;

			sector.save(self.buffer, &transaction).await?;

			if self.buffer == SectorBuffer::Initial {
				drop(sector);
				self.sectors.evict_sector(sector_index).await;
			}
		}

		transaction.commit().await
			.map(|_| written)
			.map_err(IoError::from)
	}

	async fn flush(&mut self) -> std::result::Result<(), Self::Error> {
		Ok(())
	}
}
