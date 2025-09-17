use std::collections::HashMap;
use std::sync::Arc;

use sea_orm::{ConnectionTrait, TransactionTrait, StreamTrait};
use tokio::sync::*;

use crate::board::sector::{Sector, SectorBuffer};
use crate::board::Shape;
use crate::database::{BoardsConnection, BoardsConnectionGeneric, BoardsDatabase, BoardsDatabaseError, Database};

use super::SectorAccessor;

pub struct SectorRequest {
	sector_index: usize,
	responder: oneshot::Sender<Arc<Result<Option<Sector>, BoardsDatabaseError>>>,
}

pub struct BufferedSectorCache {
	pub cache: Arc<SectorCache>,
	readback_sender: mpsc::Sender<SectorRequest>,
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
	) -> Arc<Result<Option<Sector>, BoardsDatabaseError>> {
		let (responder, reciever) = oneshot::channel();
		let request = SectorRequest { sector_index, responder };
		self.readback_sender.send(request).await.unwrap();
		reciever.await.unwrap()
	}
	
	async fn readback_thread(
		cache: Arc<SectorCache>,
		pool: Arc<BoardsDatabase>,
		mut request_receiver: mpsc::Receiver<SectorRequest>,
	) {
		let mut buffer = vec![];
		while request_receiver.recv_many(&mut buffer, 1000).await > 0 {
			let requests = buffer.drain(..);
			let mut sectors = HashMap::new();
			let connection = pool.connection().await.unwrap();
			
			for SectorRequest { sector_index, responder } in requests {
				if !sectors.contains_key(&sector_index) {
					let sector = cache.get_sector(sector_index, &connection).await
						.map(|option| option.map(|s| Sector::clone(&*s)));
					sectors.insert(sector_index, Arc::new(sector));
				}
				
				let sector = sectors.get(&sector_index).unwrap();
				let _ = responder.send(sector.clone());
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

	async fn cache_sector<C: ConnectionTrait + TransactionTrait + StreamTrait>(
		&self,
		sector_index: usize,
		connection: &BoardsConnectionGeneric<C>,
	) -> Result<RwLockWriteGuard<Option<Sector>>, BoardsDatabaseError> {
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

	pub async fn get_sector(
		&self,
		sector_index: usize,
		connection: &BoardsConnection,
	) -> Result<Option<RwLockReadGuard<Sector>>, BoardsDatabaseError> {
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

	pub async fn get_sector_mut<C: ConnectionTrait + TransactionTrait + StreamTrait>(
		&self,
		sector_index: usize,
		connection: &BoardsConnectionGeneric<C>,
	) -> Result<Option<RwLockMappedWriteGuard<Sector>>, BoardsDatabaseError> {
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
