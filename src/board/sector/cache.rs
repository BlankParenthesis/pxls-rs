use sea_orm::{DbErr, ConnectionTrait, TransactionTrait};
use tokio::sync::*;
use crate::{
	database::{BoardsConnection, BoardsConnectionGeneric},
	board::sector::{Sector, SectorBuffer},
};
use super::SectorAccessor;

pub struct SectorCache {
	// TODO: evict based on size
	board_id: i32,
	sector_size: usize,
	sectors: Vec<RwLock<Option<Sector>>>,
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

	pub fn total_sectors(&self) -> usize {
		self.sectors.len()
	}

	pub fn sector_size(&self) -> usize {
		self.sector_size
	}

	pub fn total_size(&self) -> usize {
		self.sector_size() * self.total_sectors()
	}

	// TODO: maybe a better name? this fills the cache entry, not the sector itself
	async fn fill_sector<C: ConnectionTrait + TransactionTrait>(
		&self,
		sector_index: usize,
		connection: &BoardsConnectionGeneric<C>,
	) -> Result<RwLockWriteGuard<Option<Sector>>, DbErr> {
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

	// TODO: rename to get_sector for consistency
	pub async fn read_sector(
		&self,
		sector_index: usize,
		connection: &BoardsConnection,
	) -> Result<Option<RwLockReadGuard<Sector>>, DbErr> {
		if let Some(lock) = self.sectors.get(sector_index) {
			let option = lock.read().await;
			if option.is_some() {
				Ok(Some(RwLockReadGuard::map(option, |o| o.as_ref().unwrap())))
			} else {
				drop(option);

				let sector = self.fill_sector(sector_index, connection).await?;

				Ok(Some(RwLockReadGuard::map(
					RwLockWriteGuard::downgrade(sector),
					|o| o.as_ref().unwrap(),
				)))
			}
		} else {
			Ok(None)
		}
	}

	// TODO: rename to get_sector_mut for consistency
	pub async fn write_sector<C: ConnectionTrait + TransactionTrait>(
		&self,
		sector_index: usize,
		connection: &BoardsConnectionGeneric<C>,
	) -> Option<RwLockMappedWriteGuard<Sector>> {
		if let Some(lock) = self.sectors.get(sector_index) {
			let option = lock.write().await;
			if option.is_some() {
				Some(RwLockWriteGuard::map(option, |o| o.as_mut().unwrap()))
			} else {
				drop(option);

				let sector = self.fill_sector(sector_index, connection)
					.await.unwrap();

				Some(RwLockWriteGuard::map(sector, |o| o.as_mut().unwrap()))
			}
		} else {
			None
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
