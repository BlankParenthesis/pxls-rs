use std::collections::{hash_map, HashMap};
use std::io::Write;
use std::sync::Arc;

use flate2::Compression;
use flate2::write::GzEncoder;
use enumset::EnumSet;
use reqwest::StatusCode;
use sea_orm::{ConnectionTrait, TransactionTrait, StreamTrait};
use tokio::sync::*;

use crate::board::sector::{BufferRead, Change, Sector, SectorBuffer};
use crate::database::{BoardsConnection, BoardsConnectionGeneric, BoardsDatabase, DatabaseError, Database};

use super::SectorAccessor;

pub struct SectorRequest {
	sector_index: usize,
	sector_type: SectorBuffer,
	// TODO: terrible error type but we can't clone BoardsDatabaseError
	responder: oneshot::Sender<Result<Option<CompressedSector>, StatusCode>>,
}

pub struct BufferedSectorCache {
	pub cache: Arc<SectorCache>,
	readback_sender: mpsc::Sender<SectorRequest>,
}

#[derive(Clone)]
pub struct CompressedSector {
	pub raw: Vec<u8>,
	pub compressed: Vec<u8>,
}

impl CompressedSector {
	fn compress(data: &[u8]) -> Vec<u8> {
		let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
		encoder.write_all(data).unwrap();
		encoder.finish().unwrap()
	}
	
	fn new(data: bytes::BytesMut) -> Self {
		let raw = data.to_vec();
		let compressed = Self::compress(&raw);
		Self { raw, compressed }
	}
	
	fn update(&mut self, delta: Vec<Change>) {
		if !delta.is_empty() {
			for Change { position, data } in delta {
				self.raw[position] = data;
			}
			
			self.compressed = Self::compress(&self.raw);
		}
	}
}

#[derive(Default)]
pub struct BufferedSector {
	pub colors: Option<CompressedSector>,
	pub timestamps: Option<CompressedSector>,
	pub initial: Option<CompressedSector>,
	pub mask: Option<CompressedSector>,
}

impl BufferedSector {
	fn get(&self, buffer: SectorBuffer) -> &Option<CompressedSector> {
		match buffer {
			SectorBuffer::Colors => &self.colors,
			SectorBuffer::Timestamps => &self.timestamps,
			SectorBuffer::Initial => &self.initial,
			SectorBuffer::Mask => &self.mask,
			SectorBuffer::Density => &None,
		}
	}
}

impl BufferedSectorCache {
	pub fn new(
		board_id: i32,
		sector_count: usize,
		sector_size: usize,
		pool: Arc<BoardsDatabase>,
	) -> Self {
		let cache = Arc::new(SectorCache::new(board_id, sector_count, sector_size));
		
		let (readback_sender, readback_reciever) = mpsc::channel(1000);
		tokio::spawn(Self::readback_thread(cache.clone(), pool, readback_reciever));
		
		Self { cache, readback_sender }
	}
	
	pub async fn get_sector(
		&self,
		sector_index: usize,
		sector_type: SectorBuffer,
	) -> Result<Option<CompressedSector>, StatusCode> {
		let (responder, reciever) = oneshot::channel();
		let request = SectorRequest { sector_index, responder, sector_type };
		self.readback_sender.send(request).await.unwrap();
		reciever.await.unwrap()
	}
	
	async fn readback_thread(
		cache: Arc<SectorCache>,
		pool: Arc<BoardsDatabase>,
		mut request_receiver: mpsc::Receiver<SectorRequest>,
	) {
		let mut readback_buffers = vec![];
		readback_buffers.resize_with(cache.total_sectors(), BufferedSector::default);
		
		let mut buffer = vec![];
		while request_receiver.recv_many(&mut buffer, 1000).await > 0 {
			let requests = std::mem::take(&mut buffer);
			let connection = pool.connection().await.unwrap();
			let mut requested_sectors = HashMap::<usize, EnumSet<SectorBuffer>>::new();
			
			for SectorRequest { sector_index, sector_type, .. } in requests.iter() {
				match requested_sectors.entry(*sector_index) {
					hash_map::Entry::Occupied(mut e) => {
						e.get_mut().insert(*sector_type);
					},
					hash_map::Entry::Vacant(e) => {
						e.insert(EnumSet::from(*sector_type));
					},
				}
			};
			
			let mut sectors = HashMap::new();
			for SectorRequest { sector_index, sector_type, responder } in requests {
				if let hash_map::Entry::Vacant(e) = sectors.entry(sector_index) {
					let sector = cache.get_sector_mut(sector_index, &connection).await
						.map(|option| {
							option.map(|mut s| {
								let colors = s.colors.readback();
								let timestamps = s.timestamps.readback();
								let mask = s.mask.readback();
								let initial = s.initial.readback();
								drop(s);
								
								let buffer = readback_buffers.get_mut(sector_index).unwrap();
								
								match colors {
									BufferRead::Delta(delta) => {
										buffer.colors.as_mut().unwrap().update(delta);
									},
									BufferRead::Full(data) => {
										buffer.colors = Some(CompressedSector::new(data));
									},
								}
								match timestamps {
									BufferRead::Delta(delta) => {
										buffer.timestamps.as_mut().unwrap().update(delta);
									},
									BufferRead::Full(data) => {
										buffer.timestamps = Some(CompressedSector::new(data));
									},
								}
								match mask {
									BufferRead::Delta(delta) => {
										buffer.mask.as_mut().unwrap().update(delta);
									},
									BufferRead::Full(data) => {
										buffer.mask = Some(CompressedSector::new(data));
									},
								}
								match initial {
									BufferRead::Delta(delta) => {
										buffer.initial.as_mut().unwrap().update(delta);
									},
									BufferRead::Full(data) => {
										buffer.initial = Some(CompressedSector::new(data));
									},
								}
							})
						});
					
					e.insert(sector);
				}
				
				let sector = sectors.get(&sector_index).unwrap();
				let buffer = match sector {
					Ok(Some(())) => {
						let buffer = readback_buffers.get(sector_index).unwrap();
						Ok(buffer.get(sector_type).clone())
					},
					Ok(None) => Ok(None),
					Err(e) => Err(StatusCode::from(e)),
				};
				let _ = responder.send(buffer);
			}
		}
	}
}

pub struct SectorCache {
	// TODO: evict based on size
	board_id: i32,
	sector_size: usize,
	sectors: Vec<RwLock<Option<Sector>>>,
}

impl SectorCache {
	fn new(
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

	pub fn total_sectors(&self) -> usize {
		self.sectors.len()
	}

	pub fn sector_size(&self) -> usize {
		self.sector_size
	}

	pub fn total_size(&self) -> usize {
		self.sector_size() * self.total_sectors()
	}

	async fn cache_sector<'l, C: ConnectionTrait + TransactionTrait + StreamTrait>(
		&'l self,
		sector_index: usize,
		connection: &BoardsConnectionGeneric<C>,
	) -> Result<RwLockWriteGuard<'l, Option<Sector>>, DatabaseError> {
		let mut option = self
			.sectors
			.get(sector_index).unwrap()
			.write().await;

		let load = Sector::load(
			self.board_id,
			sector_index as i32,
			connection,
		).await?;

		let sector = match load {
			Some(sector) => sector,
			None => {
				Sector::new(
					self.board_id,
					sector_index as i32,
					self.sector_size,
					connection,
				).await?
			},
		};

		option.replace(sector);

		Ok(option)
	}

	pub async fn evict_sector(
		&self,
		sector_index: usize,
	) -> Option<Sector> {
		let mut option = self
			.sectors
			.get(sector_index).unwrap()
			.write().await;

		option.take()
	}

	pub async fn get_sector<'l>(
		&'l self,
		sector_index: usize,
		connection: &BoardsConnection,
	) -> Result<Option<RwLockReadGuard<'l, Sector>>, DatabaseError> {
		if let Some(lock) = self.sectors.get(sector_index) {
			let option = lock.read().await;
			if option.is_some() {
				Ok(Some(RwLockReadGuard::map(option, |o| o.as_ref().unwrap())))
			} else {
				drop(option);

				let sector = self.cache_sector(sector_index, connection).await?;

				Ok(Some(RwLockReadGuard::map(
					RwLockWriteGuard::downgrade(sector),
					|o| o.as_ref().unwrap(),
				)))
			}
		} else {
			Ok(None)
		}
	}

	pub async fn get_sector_mut<'l, C: ConnectionTrait + TransactionTrait + StreamTrait>(
		&'l self,
		sector_index: usize,
		connection: &BoardsConnectionGeneric<C>,
	) -> Result<Option<RwLockMappedWriteGuard<'l, Sector>>, DatabaseError> {
		if let Some(lock) = self.sectors.get(sector_index) {
			let option = lock.write().await;
			if option.is_some() {
				Ok(Some(RwLockWriteGuard::map(option, |o| o.as_mut().unwrap())))
			} else {
				drop(option);

				let sector = self.cache_sector(sector_index, connection)
					.await?;

				Ok(Some(RwLockWriteGuard::map(sector, |o| o.as_mut().unwrap())))
			}
		} else {
			Ok(None)
		}
	}

	pub fn access<'l>(
		&'l self,
		buffer: SectorBuffer,
		connection: &'l BoardsConnection,
	) -> SectorAccessor<'l> {
		SectorAccessor::new(self, buffer, connection)
	}
}
