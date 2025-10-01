mod socket;
mod cooldown;
mod sector;
mod shape;
mod activity;

use std::collections::{hash_map::Entry, HashMap, HashSet};
use std::convert::TryFrom;
use std::io::{Seek, SeekFrom};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tokio::sync::{mpsc::{self, error::SendError}, Mutex, RwLock};
use tokio::time::{Duration, Instant};
use warp::http::StatusCode;
use warp::reply::{Response, Reply};

use crate::routes::board_moderation::boards::pixels::Overrides;
use crate::routes::placement_statistics::users::PlacementColorStatistics;
use crate::routes::core::boards::pixels::PlacementFilter;
use crate::config::CONFIG;
use crate::database::{User, Database, DatabaseError, DbConn, Order, BoardInfo, BoardSpecifier, Placement, UserSpecifier, BoardNoticeSpecifier, BoardsNotice, DbInsertResult, MaskValue, Palette, PlacementSpecifier, Sector, SectorBuffer, PlacementPageToken};
use crate::filter::response::paginated_list::Page;
use crate::filter::response::reference::Reference;
use crate::filter::body::patch::BinaryPatch;
use crate::filter::header::range::Range;
use crate::AsyncWrite;

use socket::{Connections, Packet, Socket};
use sector::{SectorAccessor, BufferedSectorCache, IoError, CompressedSector};
use cooldown::CooldownInfo;

pub use activity::ActivityCache;
pub use cooldown::CooldownCache;
pub use shape::Shape;
pub use socket::BoardSubscription;

#[derive(Debug)]
pub enum PlaceError {
	UnknownMaskValue,
	Unplacable,
	InvalidColor,
	NoOp,
	Cooldown,
	OutOfBounds,
	DatabaseError(DatabaseError),
	SenderError(SendError<PendingPlacement>),
	Banned,
}

impl From<DatabaseError> for PlaceError {
	fn from(value: DatabaseError) -> Self {
		Self::DatabaseError(value)
	}
}

impl From<SendError<PendingPlacement>> for PlaceError {
	fn from(value: SendError<PendingPlacement>) -> Self {
		Self::SenderError(value)
	}
}

impl From<&PlaceError> for StatusCode {
	fn from(value: &PlaceError) -> Self {
		match value {
			PlaceError::UnknownMaskValue => {
				eprintln!("Unknown mask value for board");
				StatusCode::INTERNAL_SERVER_ERROR
			},
			PlaceError::Unplacable => StatusCode::FORBIDDEN,
			PlaceError::InvalidColor => StatusCode::UNPROCESSABLE_ENTITY,
			PlaceError::NoOp => StatusCode::CONFLICT,
			PlaceError::Cooldown => StatusCode::TOO_MANY_REQUESTS,
			PlaceError::OutOfBounds => StatusCode::NOT_FOUND,
			PlaceError::DatabaseError(_) | PlaceError::SenderError(_) => {
				StatusCode::INTERNAL_SERVER_ERROR
			},
			PlaceError::Banned => StatusCode::FORBIDDEN,
		}
	}
}

impl From<PlaceError> for StatusCode {
	fn from(value: PlaceError) -> Self {
		StatusCode::from(&value)
	}
}


#[derive(Debug)]
pub enum UndoError {
	OutOfBounds,
	WrongUser,
	Expired,
	DatabaseError(DatabaseError),
	Banned,
}

impl From<DatabaseError> for UndoError {
	fn from(value: DatabaseError) -> Self {
		Self::DatabaseError(value)
	}
}


impl From<&UndoError> for StatusCode {
	fn from(value: &UndoError) -> Self {
		match value {
			UndoError::OutOfBounds => StatusCode::NOT_FOUND,
			UndoError::WrongUser => StatusCode::FORBIDDEN,
			UndoError::Expired => StatusCode::CONFLICT,
			UndoError::DatabaseError(_) => {
				StatusCode::INTERNAL_SERVER_ERROR
			},
			UndoError::Banned => StatusCode::FORBIDDEN,
		}
	}
}

impl From<UndoError> for StatusCode {
	fn from(value: UndoError) -> Self {
		StatusCode::from(&value)
	}
}

impl Reply for UndoError {
	fn into_response(self) -> Response {
		StatusCode::from(&self).into_response()
	}
}


#[derive(Debug)]
pub enum PatchError {
	SeekFailed(IoError),
	WriteFailed(IoError),
	WriteOutOfBounds,
}


impl From<&PatchError> for StatusCode {
	fn from(value: &PatchError) -> Self {
		match value {
			PatchError::SeekFailed(_) => StatusCode::CONFLICT,
			PatchError::WriteFailed(_) => StatusCode::INTERNAL_SERVER_ERROR,
			PatchError::WriteOutOfBounds => StatusCode::CONFLICT,
		}
	}
}

impl From<PatchError> for StatusCode {
	fn from(value: PatchError) -> Self {
		StatusCode::from(&value)
	}
}


impl Reply for PatchError {
	fn into_response(self) -> Response {
		StatusCode::from(&self).into_response()
	}
}

// TODO: This was generic but self referencing things do not like generics so it
// has been made concrete here. That said, it's ugly and it would be nice to
// refactor it if possible:

use ouroboros::self_referencing;

#[derive(Default)]
pub struct StatisticsHashLock<K> {
	locks: Mutex<HashMap<K, Arc<Mutex<PlacementColorStatistics>>>>,
}

#[self_referencing]
pub struct StatisticsHashLockGuard {
	lock: Arc<Mutex<PlacementColorStatistics>>,
	#[covariant]
	#[borrows(lock)]
	guard: tokio::sync::MutexGuard<'this, PlacementColorStatistics>,
}

impl std::ops::Deref for StatisticsHashLockGuard {
	type Target = PlacementColorStatistics;

	fn deref(&self) -> &Self::Target {
		self.borrow_guard().deref()
	}
}

impl std::ops::DerefMut for StatisticsHashLockGuard {
	fn deref_mut(&mut self) -> &mut Self::Target {
		self.with_guard_mut(|v| v.deref_mut())
	}
}

impl<T: Eq + PartialEq + std::hash::Hash> StatisticsHashLock<T> {
	async fn lock(&self, key: T) -> StatisticsHashLockGuard {
		use std::collections::hash_map;
		let mut locks = self.locks.lock().await;
		let lock = match locks.entry(key) {
			hash_map::Entry::Occupied(o) => o.get().clone(),
			hash_map::Entry::Vacant(v) => v.insert(Default::default()).clone(),
		};
		StatisticsHashLockGuardAsyncSendBuilder {
			lock,
			guard_builder: |lock| Box::pin(lock.lock())
		}.build().await
	}
}

impl<K: Eq + PartialEq + std::hash::Hash> From<HashMap<K, PlacementColorStatistics>> for StatisticsHashLock<K> {
	fn from(value: HashMap<K, PlacementColorStatistics>) -> Self {
		let map = value.into_iter().map(|(k, v)| {
			let value = std::sync::Arc::new(tokio::sync::Mutex::new(v));
			(k, value)
		}).collect();
		let locks = tokio::sync::Mutex::new(map);
		Self { locks }
	}
}

#[derive(Debug, Clone)]
pub struct PendingPlacement {
	pub position: u64,
	pub color: u8,
	pub timestamp: u32,
	pub user: UserSpecifier,
}

pub struct Board {
	pub info: BoardInfo,
	connections: RwLock<Connections>,
	sectors: BufferedSectorCache,
	statistics_cache: StatisticsHashLock<UserSpecifier>,
	activity_cache: Mutex<ActivityCache>,
	cooldown_cache: RwLock<CooldownCache>,
	placement_sender: mpsc::Sender<PendingPlacement>,
	pool: Arc<Database>,
	lookup_cache: RwLock<HashMap<u64, Option<Placement>>>,
}

impl Serialize for Board {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where S: serde::Serializer {
		self.info.serialize(serializer)
	}
}

impl Board {
	pub fn new(
		info: BoardInfo,
		statistics_cache: StatisticsHashLock<UserSpecifier>,
		activity_cache: Mutex<ActivityCache>,
		cooldown_cache: RwLock<CooldownCache>,
		pool: Arc<Database>,
	) -> Self {

		let sectors = BufferedSectorCache::new(
			info.id,
			info.shape.sector_count(),
			info.shape.sector_size(),
			pool.clone(),
		);

		let connections = RwLock::new(Connections::default());
		
		let (placement_sender, placement_receiver) = mpsc::channel(10000);

		tokio::spawn(Self::buffer_placements(info.id, placement_receiver, pool.clone()));
		
		let lookup_cache = RwLock::new(HashMap::new());

		Self {
			info,
			sectors,
			connections,
			statistics_cache,
			activity_cache,
			cooldown_cache,
			placement_sender,
			pool,
			lookup_cache,
		}
	}
	
	async fn buffer_placements(
		board: BoardSpecifier,
		mut receiver: mpsc::Receiver<PendingPlacement>,
		pool: Arc<Database>,
	) {
		let mut buffer = vec![];
		let tick_time = Duration::from_millis(
			CONFIG.database_tickrate.map(|r| (1000.0 / r) as u64).unwrap_or(0)
		);
		let mut next_tick = Instant::now() + tick_time;
		while receiver.recv_many(&mut buffer, 10000).await > 0 {
			let connection = pool.connection().await
				.expect("A board database insert thread failed to connect to the database");
			connection.create_placements(board.into(), buffer.drain(..).as_slice()).await
				.expect("A board database insert thread failed to insert placements");
			
			tokio::time::sleep_until(next_tick).await;
			next_tick = Instant::now() + tick_time;
		}
	}
	
	pub fn reference(&self) -> Reference<BoardInfo> {
		Reference::from(self.info.clone())
	}

	pub async fn read<'l>(
		&'l self,
		buffer: SectorBuffer,
		connection: &'l DbConn,
	) -> SectorAccessor<'l> {
		self.sectors.cache.access(buffer, connection)
	}
	
	pub async fn try_read_exact_sector(
		&self,
		range: Range,
		sector_type: SectorBuffer,
		timestamps: bool,
	) -> Option<(Result<Option<CompressedSector>, StatusCode>, std::ops::Range<usize>)> {
		let multiplier = if timestamps { 4 } else { 1 };
		
		let range = match range {
			Range::Single { unit, range } if unit.to_lowercase() == "bytes" => {
				range.with_length(self.info.shape.total_size() * multiplier).ok()
			},
			_ => None,
		}?;
		
		let sector_size = self.info.shape.sector_size() * multiplier;
		
		let sector_aligned = range.start % sector_size == 0;
		let sector_sized = range.len() == sector_size;
		
		if sector_aligned && sector_sized {
			let sector_index = range.start / sector_size;
			Some((self.sectors.get_sector(sector_index, sector_type).await, range))
		} else {
			None
		}
	}
	
	async fn try_patch(
		&self,
		// NOTE: can only patch initial or mask
		buffer: SectorBuffer,
		patch: &BinaryPatch,
		connection: &DbConn,
	) -> Result<(), PatchError> {

		let end = patch.start + patch.data.len();
		let total_pixels = self.sectors.cache.total_size();
		if end > total_pixels {
			return Err(PatchError::WriteOutOfBounds);
		}

		let mut sector_data = self.sectors.cache.access(buffer, connection);

		sector_data
			.seek(SeekFrom::Start(u64::try_from(patch.start).unwrap()))
			.map_err(IoError::Io)
			.map_err(PatchError::SeekFailed)?;

		sector_data
			.write(&patch.data).await
			.map_err(PatchError::WriteFailed)?;

		let change = socket::Change {
			position: u64::try_from(patch.start).unwrap(),
			values: Vec::from(&*patch.data),
		};

		let mut packet = socket::BoardData::builder();
		
		match buffer {
			SectorBuffer::Initial => packet = packet.initial(vec![change]),
			SectorBuffer::Mask => packet = packet.mask(vec![change]),
			_ => panic!("cannot patch colors/timestamps")
		}
		
		let connections = self.connections.read().await;
		connections.queue_board_change(packet).await;

		Ok(())
	}

	pub async fn try_patch_initial(
		&self,
		patch: &BinaryPatch,
		connection: &DbConn,
	) -> Result<(), PatchError> {
		self.try_patch(SectorBuffer::Initial, patch, connection).await
	}

	pub async fn try_patch_mask(
		&self,
		patch: &BinaryPatch,
		connection: &DbConn,
	) -> Result<(), PatchError> {
		self.try_patch(SectorBuffer::Mask, patch, connection).await
	}

	pub async fn update_info(
		&mut self,
		name: Option<String>,
		shape: Option<Vec<Vec<usize>>>,
		palette: Option<Palette>,
		max_pixels_available: Option<u32>,
		connection: &DbConn,
	) -> Result<(), DatabaseError> {
		assert!(
			name.is_some()
			|| palette.is_some()
			|| shape.is_some()
			|| max_pixels_available.is_some()
		);

		connection.edit_board(
			self.info.id,
			name.clone(),
			shape.clone(),
			palette.clone(),
			max_pixels_available,
		).await?;

		let shape = shape.map(Shape::new);


		if let Some(ref name) = name {
			self.info.name = name.clone();
		}

		if let Some(ref palette) = palette {
			self.info.palette = palette.clone();
		}

		if let Some(ref shape) = shape {
			self.info.shape = shape.clone();

			self.sectors = BufferedSectorCache::new(
				self.info.id,
				self.info.shape.sector_count(),
				self.info.shape.sector_size(),
				self.pool.clone(),
			)
		}

		if let Some(max_stacked) = max_pixels_available {
			self.info.max_pixels_available = max_stacked;
		}

		let packet = Packet::BoardUpdate {
			info: Some(socket::BoardInfo {
				name,
				shape,
				palette,
				max_pixels_available,
			}),
			data: None,
		};
		
		let connections = self.connections.read().await;
		
		let users = if max_pixels_available.is_some() {
			connections.users()
		} else {
			vec![]
		};
		
		let mut cooldowns = Vec::with_capacity(users.len());
		for user in users {
			let cooldown = self.user_cooldown_info(user).await?;
			cooldowns.push((user, cooldown));
		}

		connections.send(packet).await;

		for (user, cooldown) in cooldowns {
			connections.set_user_cooldown(user, cooldown).await;
		}

		Ok(())
	}

	pub async fn delete(
		self,
		connection: &DbConn,
	) -> Result<(), DatabaseError> {
		let mut connections = self.connections.write().await;
		connections.close();
		connection.delete_board(self.info.id).await
	}

	fn check_placement_palette(
		&self,
		color: u8,
		color_override: bool,
	) -> Result<(), PlaceError> {
		let palette_entry = self.info.palette.get(&(color as u32));
		match palette_entry {
			Some(color) => {
				if !color.system_only || color_override {
					Ok(())
				} else {
					Err(PlaceError::Unplacable)
				}
			},
			None => Err(PlaceError::InvalidColor),
		}
	}
	
	async fn insert_placement(
		&self,
		user: &User,
		position: u64,
		color: u8,
		timestamp: u32,
		sector_offset: usize,
		sector: &mut Sector,
	) -> Result<Placement, PlaceError> {
		let mut cooldown_cache = self.cooldown_cache.write().await;
		let mut activity_cache = self.activity_cache.lock().await;
		let user_specifier = *user.specifier();
		
		let pending_placement = PendingPlacement {
			user: user_specifier,
			timestamp,
			position,
			color,
		};
		self.placement_sender.send(pending_placement).await?;
		
		let new_placement = Placement {
			position,
			color,
			modified: timestamp,
			user: Reference::from(user.clone()),
		};
		let mut lookup_cache = self.lookup_cache.write().await;
		lookup_cache.insert(position, Some(new_placement.clone()));
		drop(lookup_cache);
		
		activity_cache.insert(new_placement.modified, user_specifier);
		
		let activity = activity_cache.count(new_placement.modified) as u32;
		let density = sector.density.read_u32(sector_offset) + 1;
		
		cooldown_cache.insert(new_placement.modified, user_specifier, activity, density);
		
		sector.colors.write(sector_offset, color);
		sector.timestamps.write_u32(sector_offset, timestamp);
		sector.density.write_u32(sector_offset, density);
		
		let data = socket::BoardData::builder()
			.colors(vec![socket::Change { position, values: vec![color] }])
			.timestamps(vec![socket::Change { position, values: vec![timestamp] }]);

		let connections = self.connections.read().await;
		connections.queue_board_change(data).await;
		
		Ok(new_placement)
	}

	// Returns a tuple of:
	// - the number of placements that changed the board 
	// - the timestamp they were placed at
	pub async fn mass_place(
		&self,
		user: &User,
		placements: &[(u64, u8)],
		overrides: Overrides,
		connection: &DbConn,
	) -> Result<(usize, u32), PlaceError> {
		let user_specifier = user.specifier();
		
		if connection.is_user_banned(user_specifier).await? {
			return Err(PlaceError::Banned);
		}

		// preliminary checks and mapping to sector local values
		let sector_placements = placements.iter()
			.copied()
			.map(|(position, color)| {
				self.check_placement_palette(color, overrides.color)
					.and_then(|()| {
						self.info.shape.to_local(position as usize)
							.ok_or(PlaceError::OutOfBounds)
					})
					.map(|(s_i, s_o)| (position, (s_i, s_o), color))
			})
			.collect::<Result<Vec<_>, _>>()?;

		let used_sectors = sector_placements.iter()
			.map(|(_, (i, _), _)| *i)
			.collect::<HashSet<_>>();

		// lock all the relevant sectors
		let mut sectors = HashMap::new();
		for sector_index in used_sectors {
			if let Entry::Vacant(vacant) = sectors.entry(sector_index) {
				let sector =  self.sectors.cache
					.get_sector_mut(sector_index, connection).await?
					.expect("Missing sector");

				vacant.insert(sector);
			}
		}

		let mut changes: usize = 0;
		// final checks
		for (_, (sector_index, sector_offset), color) in sector_placements.iter() {
			let sector = sectors.get(sector_index).unwrap();

			if !overrides.mask {
				match MaskValue::try_from(sector.mask.read(*sector_offset)).ok() {
					Some(MaskValue::Place) => Ok(()),
					Some(MaskValue::NoPlace) => Err(PlaceError::Unplacable),
					Some(MaskValue::Adjacent) => unimplemented!(),
					None => Err(PlaceError::UnknownMaskValue),
				}?;
			}

			if sector.colors.read(*sector_offset) != *color {
				changes += 1;
			}
		}

		if changes == 0 {
			return Err(PlaceError::NoOp);
		}

		// This acts as a per-user lock to prevent exploits bypassing cooldown
		let mut statistics_lock = self.statistics_cache.lock(*user_specifier).await;

		if !overrides.cooldown {
			let cooldown_info = self.user_cooldown_info(user_specifier).await?;

			if (cooldown_info.pixels_available as usize) < changes {
				return Err(PlaceError::Cooldown);
			}
		}

		let timestamp = self.current_timestamp();
		
		for (position, (sector_index, sector_offset), color) in sector_placements {
			let sector = sectors.get_mut(&sector_index).unwrap();

			self.insert_placement(
				user,
				position,
				color,
				timestamp,
				sector_offset,
				sector,
			).await?;
			
			statistics_lock.colors.entry(color).or_default().placed += 1;
		}

		let cooldown_info = self.user_cooldown_info(user_specifier).await?; // TODO: maybe cooldown err instead

		let connections = self.connections.read().await;
		connections.set_user_cooldown(user_specifier, cooldown_info.clone()).await;

		let stats = statistics_lock.clone();
		connections.send(socket::Packet::BoardStatsUpdated { stats }).await;

		Ok((changes, timestamp))
	}

	pub async fn try_undo(
		&self,
		user: &User,
		placement: &PlacementSpecifier,
		connection: &DbConn,
	) -> Result<CooldownInfo, UndoError> {
		let position = placement.position();
		
		let user_specifier = user.specifier();
		if connection.is_user_banned(user_specifier).await? {
			return Err(UndoError::Banned);
		}
		
		let (sector_index, sector_offset) = self.info.shape
			.to_local(position as usize)
			.ok_or(UndoError::OutOfBounds)?;

		let mut sector = self.sectors.cache
			.get_sector_mut(sector_index, connection).await?
			.expect("Missing sector");

		// This acts as a per-user lock to prevent exploits bypassing cooldown
		let mut statistics_lock = self.statistics_cache.lock(*user_specifier).await;

		let transaction = connection.begin().await?;
		
		let (undone_placement, last_placement) = transaction
			.get_two_placements(placement).await?;

		let placement_id = match undone_placement {
			Some(placement) if placement.user == *user_specifier => {
				let deadline = placement.modified + CONFIG.undo_deadline_seconds;
				if deadline < self.current_timestamp() {
					return Err(UndoError::Expired)
				}
				placement.id
			}
			_ => return Err(UndoError::WrongUser)
		};

		transaction.delete_placement(placement_id).await?;
		let mut lookup_cache = self.lookup_cache.write().await;
		lookup_cache.remove(&position);
		drop(lookup_cache);

		let (color, timestamp) = match last_placement {
			Some(placement) => (placement.color, placement.modified),
			None => (sector.initial.read(sector_offset), 0),
		};
		let density = sector.density.read_u32(sector_offset).checked_sub(1).unwrap();

		sector.colors.write(sector_offset, color);
		sector.timestamps.write_u32(sector_offset, timestamp);
		sector.density.write_u32(sector_offset, density);

		transaction.commit().await?;

		let mut cooldown_cache = self.cooldown_cache.write().await;
		let mut activity_cache = self.activity_cache.lock().await;
		activity_cache.remove(timestamp, *user_specifier);
		cooldown_cache.remove(timestamp, *user_specifier);
		drop(cooldown_cache);
		drop(activity_cache);

		statistics_lock.colors.entry(color).or_default().placed -= 1;

		let color = socket::Change {
			position,
			values: vec![color],
		};

		let timestamp = socket::Change {
			position,
			values: vec![timestamp],
		};

		let data = socket::BoardData::builder()
			.colors(vec![color])
			.timestamps(vec![timestamp]);

		let cooldown_info = self.user_cooldown_info(user_specifier).await?;
		let connections = self.connections.read().await;
		connections.queue_board_change(data).await;
		connections.set_user_cooldown(user_specifier, cooldown_info.clone()).await;

		let stats = statistics_lock.clone();
		connections.send(socket::Packet::BoardStatsUpdated { stats }).await;
		
		Ok(cooldown_info)
	}

	// TODO: re-evaluate anonymous placing, maybe try and implement it again
	pub async fn try_place(
		&self,
		user: &User,
		placement: &PlacementSpecifier,
		color: u8,
		overrides: Overrides,
		connection: &DbConn,
	) -> Result<(CooldownInfo, Placement), PlaceError> {
		let user_specifier = user.specifier();
		if connection.is_user_banned(user_specifier).await? {
			return Err(PlaceError::Banned);
		}
		
		let position = placement.position();

		let (sector_index, sector_offset) = self.info.shape
			.to_local(position as usize)
			.ok_or(PlaceError::OutOfBounds)?;

		self.check_placement_palette(color, overrides.color)?;
		
		let mut sector = self.sectors.cache
			.get_sector_mut(sector_index, connection).await?
			.expect("Missing sector");

		if !overrides.mask {
			match MaskValue::try_from(sector.mask.read(sector_offset)).ok() {
				Some(MaskValue::Place) => Ok(()),
				Some(MaskValue::NoPlace) => Err(PlaceError::Unplacable),
				// NOTE: there exists an old implementation in the version
				// control history. It's messy and would need to load adjacent
				// sectors now so I'm dropping it for now.
				Some(MaskValue::Adjacent) => unimplemented!(),
				None => Err(PlaceError::UnknownMaskValue),
			}?;
		}

		if sector.colors.read(sector_offset) == color {
			return Err(PlaceError::NoOp);
		}

		// This acts as a per-user lock to prevent exploits bypassing cooldown
		let mut statistics_lock = self.statistics_cache.lock(*user_specifier).await;

		let timestamp = self.current_timestamp();
		// TODO: ignore cooldown should probably also mark the pixel as not
		// contributing to the pixels available
		if !overrides.cooldown {
			let cooldown_info = self.user_cooldown_info(user_specifier).await?;

			if cooldown_info.pixels_available == 0 {
				return Err(PlaceError::Cooldown);
			}
		}

		let new_placement = self.insert_placement(
			user,
			position,
			color,
			timestamp,
			sector_offset,
			&mut sector,
		).await?;
		drop(sector);

		statistics_lock.colors.entry(color).or_default().placed += 1;

		// TODO: maybe cooldown err instead
		let cooldown_info = self.user_cooldown_info(user_specifier).await?;
		let connections = self.connections.read().await;
		connections.set_user_cooldown(user_specifier, cooldown_info.clone()).await;

		let stats = statistics_lock.clone();
		connections.send(socket::Packet::BoardStatsUpdated { stats }).await;

		Ok((cooldown_info, new_placement))
	}

	pub async fn list_placements(
		&self,
		token: PlacementPageToken,
		limit: usize,
		order: Order,
		filter: PlacementFilter,
		connection: &DbConn,
	) -> Result<Page<Placement>, DatabaseError> {
		connection.list_placements(
			&self.info.id.into(),
			token,
			limit,
			order,
			filter,
		).await
	}

	pub async fn lookup(
		&self,
		placement: &PlacementSpecifier,
		connection: &DbConn,
	) -> Result<Option<Placement>, DatabaseError> {
		let position = placement.position();
		// TODO: rather than this, a userid buffer would allow precached lookups
		let cache = self.lookup_cache.read().await;
		if cache.contains_key(&position) {
			Ok(cache.get(&position).unwrap().clone())
		} else {
			// Avoid keeping this locked while we do the lookup since it can block placing
			drop(cache);
			
			let placement = connection.get_placement(placement).await?;
			let mut cache = self.lookup_cache.write().await;
			
			// Because we dropped the lock, there's a chance this function was
			// called elsewhere and obtained a different result. To avoid populating
			// the cache with wrong information, the cache should only be updated
			// if the entry doesn't exist or is older than the new information.
			// TODO: consider undos
			match cache.entry(position) {
				Entry::Occupied(e) => {
					if let (Some(cached), Some(lookup)) = (e.get(), &placement) {
						if cached.modified < lookup.modified {
							cache.insert(position, placement.clone());
						}
					}
				},
				Entry::Vacant(e) => {
					e.insert(placement.clone());
				},
			}
			Ok(placement)
		}
	}

	fn current_timestamp(&self) -> u32 {
		let unix_time = SystemTime::now()
			.duration_since(UNIX_EPOCH)
			.unwrap()
			.as_secs();

		u32::try_from(
			unix_time
				.saturating_sub(self.info.created_at)
				.max(1),
		)
		.unwrap()
	}

	pub async fn user_cooldown_info(
		&self,
		user: &UserSpecifier,
	) -> Result<CooldownInfo, DatabaseError> {
		let cache = self.cooldown_cache.read().await;
		Ok(cache.get(*user, self.current_timestamp()))
	}

	pub async fn user_count(&self) -> usize {
		let mut cache = self.activity_cache.lock().await;
		cache.count(self.current_timestamp())
	}

	// TODO: make configurable
	pub fn idle_timeout(&self) -> u32 {
		5 * 60
	}

	pub async fn insert_socket(
		&self,
		socket: &Arc<Socket>,
	) -> Result<(), DatabaseError> {
		let user = socket.user().await;
		// TODO: is this needed?
		let cooldown_info = if let Some(user) = user {
			Some(self.user_cooldown_info(&user).await?)
		} else {
			None
		};
		
		let mut connections = self.connections.write().await;
		connections.insert(socket, cooldown_info).await;

		Ok(())
	}

	pub async fn remove_socket(
		&self,
		socket: &Arc<Socket>,
	) {
		let mut connections = self.connections.write().await;
		connections.remove(socket).await;
	}

	pub async fn create_notice(
		&self,
		title: String,
		content: String,
		expiry: Option<u64>,
		user: Option<&User>,
		connection: &DbConn,
	) -> DbInsertResult<Reference<BoardsNotice>> {
		let notice = connection.create_board_notice(
			self.info.id.into(),
			title,
			content,
			expiry,
			user.map(|u| u.specifier()),
		).await?;
		
		let reference = Reference::from(notice);

		let packet = Packet::BoardNoticeCreated {
			notice: reference.clone(),
		};

		let connections = self.connections.read().await;
		connections.send(packet).await;

		Ok(reference)
	}

	pub async fn edit_notice(
		&self,
		notice: &BoardNoticeSpecifier,
		title: Option<String>,
		content: Option<String>,
		expiry: Option<Option<u64>>,
		connection: &DbConn,
	) -> Result<Reference<BoardsNotice>, DatabaseError> {
		let notice = connection.edit_board_notice(
			notice,
			title,
			content,
			expiry,
		).await?;

		let packet = Packet::BoardNoticeUpdated {
			notice: notice.clone(),
		};

		let connections = self.connections.read().await;
		connections.send(packet).await;

		Ok(notice)
	}

	pub async fn delete_notice(
		&self,
		notice: BoardNoticeSpecifier,
		connection: &DbConn,
	) -> Result<bool, DatabaseError> {
		let deleted = connection.delete_board_notice(&notice).await?;

		if deleted {
			let packet = Packet::BoardNoticeDeleted { notice };
	
			let connections = self.connections.read().await;
			connections.send(packet).await;
		}

		Ok(deleted)
	}

	pub async fn user_stats(
		&self,
		user: &User,
	) -> Result<PlacementColorStatistics, DatabaseError> {
		Ok((*self.statistics_cache.lock(*user.specifier()).await).clone())
	}
}
