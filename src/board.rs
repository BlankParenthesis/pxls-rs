mod socket;
mod cooldown;
mod color;
mod sector;
mod shape;
mod info;
mod placement;
mod activity;

use std::{
	convert::TryFrom,
	io::{Seek, SeekFrom},
	sync::Arc,
	time::{Duration, SystemTime, UNIX_EPOCH}, collections::{HashMap, hash_map::Entry, HashSet},
};

use bytes::BufMut;
use serde::Serialize;
use tokio::sync::{Mutex, RwLock, RwLockMappedWriteGuard};
use warp::http::{StatusCode, Uri};
use warp::{reject::Reject, reply::Response, Reply};

use crate::routes::board_moderation::boards::pixels::Overrides;
use crate::routes::placement_statistics::users::PlacementColorStatistics;
use crate::routes::board_notices::boards::notices::BoardsNotice;
use crate::routes::core::boards::pixels::PlacementFilter;
use crate::config::CONFIG;
use crate::database::{UsersConnection, DatabaseError};
use crate::filter::response::{paginated_list::Page, reference::Reference};
use crate::filter::body::patch::BinaryPatch;
use crate::database::BoardsDatabaseError;
use crate::AsyncWrite;
use crate::database::{BoardsConnection, Order};

use socket::{Connections, Packet, Socket};
use sector::{SectorAccessor, SectorCache, MaskValue, IoError};
use cooldown::CooldownInfo;
use info::BoardInfo;

pub use activity::ActivityCache;
pub use color::{Color, Palette};
pub use sector::{SectorBuffer, Sector};
pub use shape::Shape;
pub use placement::{Placement, LastPlacement, PlacementPageToken, CachedPlacement};
pub use socket::BoardSubscription;

#[derive(Debug)]
pub enum PlaceError {
	UnknownMaskValue,
	Unplacable,
	InvalidColor,
	NoOp,
	Cooldown,
	OutOfBounds,
	DatabaseError(BoardsDatabaseError),
	Banned,
}

impl From<BoardsDatabaseError> for PlaceError {
	fn from(value: BoardsDatabaseError) -> Self {
		Self::DatabaseError(value)
	}
}

impl Reject for PlaceError {}

impl Reply for PlaceError {
	fn into_response(self) -> Response {
		match self {
			Self::UnknownMaskValue => {
				eprintln!("Unknown mask value for board");
				StatusCode::INTERNAL_SERVER_ERROR
			},
			Self::Unplacable => StatusCode::FORBIDDEN,
			Self::InvalidColor => StatusCode::UNPROCESSABLE_ENTITY,
			Self::NoOp => StatusCode::CONFLICT,
			Self::Cooldown => StatusCode::TOO_MANY_REQUESTS,
			Self::OutOfBounds => StatusCode::NOT_FOUND,
			Self::DatabaseError(_) => {
				StatusCode::INTERNAL_SERVER_ERROR
			},
			Self::Banned => StatusCode::FORBIDDEN,
		}
		.into_response()
	}
}

#[derive(Debug)]
pub enum UndoError {
	OutOfBounds,
	WrongUser,
	Expired,
	DatabaseError(BoardsDatabaseError),
	Banned,
}

impl From<BoardsDatabaseError> for UndoError {
	fn from(value: BoardsDatabaseError) -> Self {
		Self::DatabaseError(value)
	}
}

impl Reject for UndoError {}

impl Reply for UndoError {
	fn into_response(self) -> Response {
		match self {
			Self::OutOfBounds => StatusCode::NOT_FOUND,
			Self::WrongUser => StatusCode::FORBIDDEN,
			Self::Expired => StatusCode::CONFLICT,
			Self::DatabaseError(_) => {
				StatusCode::INTERNAL_SERVER_ERROR
			},
			Self::Banned => StatusCode::FORBIDDEN,
		}
		.into_response()
	}
}


#[derive(Debug)]
pub enum PatchError {
	SeekFailed(IoError),
	WriteFailed(IoError),
	WriteOutOfBounds,
}

impl Reply for PatchError {
	fn into_response(self) -> Response {
		match self {
			Self::SeekFailed(_) => StatusCode::CONFLICT,
			Self::WriteFailed(_) => StatusCode::INTERNAL_SERVER_ERROR,
			Self::WriteOutOfBounds => StatusCode::CONFLICT,
		}
		.into_response()
	}
}


// If we want to handle x active users we need x * stack stored pixels to
// keep their cooldown lookup in cache. Since this is by far the most
// expensive part of placing, this is highly desirable so we can spend some
// memory here.
// So for 10,000 users, a stack size of 6, and doubled for margins:
// 10_000 * 6 * 2 = 120_000
// NOTE: ~~Placements are currently 48 bytes + id Strings (~40 bytes each)
// So entries are about 90B in size and so this is 10MB of data.~~
// *CachedPlacements* are just 16 bytes so this is now less than 2MB of data.
const PLACEMENT_CACHE_SIZE: usize = 120_000;

pub struct PlacementCache<const SIZE: usize> {
	ring_buffer: Box<[CachedPlacement; SIZE]>,
	// assumes infinite size, wrapped at lookup
	position: usize,
}

impl<const SIZE: usize> Default for PlacementCache<SIZE> {
	fn default() -> Self {
		Self {
			ring_buffer: vec![CachedPlacement::default(); SIZE]
				.into_boxed_slice()
				.try_into().unwrap(),
			position: 0,
		}
	}
}

impl<const SIZE: usize> PlacementCache<SIZE> {
	fn iter(&self) -> PlacementCacheIterator<'_, SIZE> {
		PlacementCacheIterator {
			placements: self,
			offset: 0,
		}
	}

	pub fn insert(&mut self, placement: CachedPlacement) {
		self.ring_buffer[self.position % SIZE] = placement;
		self.position += 1;
	}
}

struct PlacementCacheIterator<'a, const SIZE: usize> {
	placements: &'a PlacementCache<SIZE>,
	offset: usize,
}


impl<'a, const SIZE: usize> Iterator for PlacementCacheIterator<'a, SIZE> {
	type Item = &'a CachedPlacement;

	fn next(&mut self) -> Option<Self::Item> {
		let (remaining, _) = self.size_hint();
		if remaining > 0 {
			self.offset += 1;
			let index = (self.placements.position - self.offset) % SIZE;
			Some(&self.placements.ring_buffer[index])
		} else {
			None
		}
	}

	fn size_hint(&self) -> (usize, Option<usize>) {
		let limit = usize::min(self.placements.position, SIZE);
		let remaining = limit - self.offset;
		(remaining, Some(remaining))
	}

	fn count(self) -> usize {
		self.size_hint().0
	}

	fn last(self) -> Option<Self::Item> where Self: Sized {
		let last_offset = usize::min(self.placements.position, SIZE);
		if last_offset > 0 {
			let index = (self.placements.position - (last_offset - 1)) % SIZE;
			Some(&self.placements.ring_buffer[index])
		} else {
			None
		}
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


#[derive(Debug, Default, Clone, Copy)]
struct CooldownParameters {
	activity: usize,
	density: u32,
	timestamp: u32,
}

pub struct Board {
	pub id: i32,
	pub info: BoardInfo,
	connections: Connections,
	sectors: SectorCache,
	statistics_cache: StatisticsHashLock<i32>,
	placement_cache: RwLock<PlacementCache<PLACEMENT_CACHE_SIZE>>,
	activity_cache: Mutex<ActivityCache>,
}

impl From<&Board> for Uri {
	fn from(board: &Board) -> Self {
		format!("/boards/{}", board.id)
			.parse::<Uri>()
			.unwrap()
	}
}

impl Serialize for Board {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where S: serde::Serializer {
		self.info.serialize(serializer)
	}
}

impl Board {
	pub fn new(
		id: i32,
		name: String,
		created_at: u64,
		shape: Shape,
		palette: Palette,
		max_pixels_available: u32,
		statistics_cache: StatisticsHashLock<i32>,
		placement_cache: RwLock<PlacementCache<PLACEMENT_CACHE_SIZE>>,
		activity_cache: Mutex<ActivityCache>,
	) -> Self {
		let info = BoardInfo {
			name,
			created_at,
			shape,
			palette,
			max_pixels_available,
		};

		let sectors = SectorCache::new(
			id,
			info.shape.sector_count(),
			info.shape.sector_size(),
		);

		let connections = Connections::default();

		Self {
			id,
			info,
			sectors,
			connections,
			statistics_cache,
			placement_cache,
			activity_cache,
		}
	}

	pub async fn read<'l>(
		&'l self,
		buffer: SectorBuffer,
		connection: &'l BoardsConnection,
	) -> SectorAccessor<'l> {
		self.sectors.access(buffer, connection)
	}
	
	async fn try_patch(
		&self,
		// NOTE: can only patch initial or mask
		buffer: SectorBuffer,
		patch: &BinaryPatch,
		connection: &BoardsConnection,
	) -> Result<(), PatchError> {

		let end = patch.start + patch.data.len();
		let total_pixels = self.sectors.total_size();
		if end > total_pixels {
			return Err(PatchError::WriteOutOfBounds);
		}

		let mut sector_data = self
			.sectors
			.access(buffer, connection);

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
		
		self.connections.queue_board_change(packet).await;

		Ok(())
	}

	pub async fn try_patch_initial(
		&self,
		patch: &BinaryPatch,
		connection: &BoardsConnection,
	) -> Result<(), PatchError> {
		self.try_patch(SectorBuffer::Initial, patch, connection).await
	}

	pub async fn try_patch_mask(
		&self,
		patch: &BinaryPatch,
		connection: &BoardsConnection,
	) -> Result<(), PatchError> {
		self.try_patch(SectorBuffer::Mask, patch, connection).await
	}

	pub async fn update_info(
		&mut self,
		name: Option<String>,
		shape: Option<Vec<Vec<usize>>>,
		palette: Option<Palette>,
		max_pixels_available: Option<u32>,
		connection: &BoardsConnection,
	) -> Result<(), BoardsDatabaseError> {
		assert!(
			name.is_some()
			|| palette.is_some()
			|| shape.is_some()
			|| max_pixels_available.is_some()
		);

		connection.update_board_info(
			self.id,
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

			self.sectors = SectorCache::new(
				self.id,
				self.info.shape.sector_count(),
				self.info.shape.sector_size(),
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
		
		let users = if max_pixels_available.is_some() {
			self.connections.users()
		} else {
			vec![]
		};
		
		let mut cooldowns = Vec::with_capacity(users.len());
		for user in users {
			let cooldown = self.user_cooldown_info(user, connection).await?;
			cooldowns.push((user, cooldown));
		}

		self.connections.send(packet).await;

		for (user, cooldown) in cooldowns {
			self.connections.set_user_cooldown(user, cooldown).await;
		}

		Ok(())
	}

	pub async fn delete(
		mut self,
		connection: &BoardsConnection,
	) -> Result<(), BoardsDatabaseError> {
		self.connections.close();
		connection.delete_board(self.id).await
	}

	pub async fn last_place_time(
		&self,
		user_id: &str,
		connection: &BoardsConnection,
	) -> Result<u32, BoardsDatabaseError> {
		connection.last_place_time(self.id, user_id.to_owned()).await
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

	// Returns a tuple of:
	// - the number of placements that changed the board 
	// - the timestamp they were placed at
	pub async fn mass_place(
		&self,
		user_id: &str,
		placements: &[(u64, u8)],
		overrides: Overrides,
		connection: &BoardsConnection,
	) -> Result<(usize, u32), PlaceError> {
		if connection.is_user_banned(user_id).await? {
			return Err(PlaceError::Banned)
		}

		let uid = connection.get_uid(user_id).await?;
		// preliminary checks and mapping to sector local values
		let sector_placements = placements.iter()
			.copied()
			.map(|(position, color)| {
				self.check_placement_palette(color, overrides.color)
					.and_then(|()| {
						self.info.shape.to_local(position as usize)
							.ok_or(PlaceError::OutOfBounds)
					})
					.map(|(s_i, s_o)| ((s_i, s_o), color))
			})
			.collect::<Result<Vec<_>, _>>()?;

		let used_sectors = sector_placements.iter()
			.map(|((i, _), _)| *i)
			.collect::<HashSet<_>>();
		
		let mut cache_lock = self.placement_cache.write().await;

		// lock all the relevant sectors
		let mut sectors = HashMap::new();
		for sector_index in used_sectors {
			if let Entry::Vacant(vacant) = sectors.entry(sector_index) {
				let sector =  self.sectors
					.get_sector_mut(sector_index, connection).await?
					.expect("Missing sector");

				vacant.insert(sector);
			}
		}

		let mut changes = 0;
		// final checks
		for ((sector_index, sector_offset), color) in sector_placements.iter() {
			let sector = sectors.get(sector_index).unwrap();

			if !overrides.mask {
				match MaskValue::try_from(sector.mask[*sector_offset]).ok() {
					Some(MaskValue::Place) => Ok(()),
					Some(MaskValue::NoPlace) => Err(PlaceError::Unplacable),
					Some(MaskValue::Adjacent) => unimplemented!(),
					None => Err(PlaceError::UnknownMaskValue),
				}?;
			}

			if sector.colors[*sector_offset] != *color {
				changes += 1;
			}
		}

		if changes == 0 {
			return Err(PlaceError::NoOp);
		}

		// This acts as a per-user lock to prevent exploits bypassing cooldown
		// NOTE: no longer needed as placement_cache eclipses it and is a global lock
		let mut statistics_lock = self.statistics_cache.lock(uid).await;

		if !overrides.cooldown {
			let cooldown_info = self.user_cooldown_info_cache(
				user_id,
				connection,
				&cache_lock,
			).await?;

			if cooldown_info.pixels_available < changes {
				return Err(PlaceError::Cooldown);
			}
		}

		let timestamp = self.current_timestamp();
		
		// commit the placements
		connection.insert_placements(
			self.id,
			placements,
			timestamp,
			user_id,
		).await?;

		for ((sector_index, sector_offset), color) in sector_placements {
			let sector = sectors.get_mut(&sector_index).unwrap();

			sector.colors[sector_offset] = color;
			
			let timestamp_slice =
				&mut sector.timestamps[(sector_offset * 4)..((sector_offset + 1) * 4)];
			
			timestamp_slice
				.as_mut()
				.put_u32_le(timestamp);
			
			statistics_lock.colors.entry(color).or_default().placed += 1;
		}
		
		let mut activity_cache = self.activity_cache.lock().await;
		for &(position, _) in placements {
			cache_lock.insert(CachedPlacement {
				position,
				modified: timestamp,
				user_id: uid,
			});

			activity_cache.insert(timestamp, uid);
		}

		let mut colors = vec![];
		let mut timestamps = vec![];

		for (position, color) in placements {
			colors.push(socket::Change {
				position: *position,
				values: vec![*color],
			});

			timestamps.push(socket::Change {
				position: *position,
				values: vec![timestamp],
			});
		}

		let data = socket::BoardData::builder()
			.colors(colors)
			.timestamps(timestamps);

		self.connections.queue_board_change(data).await;

		let cooldown_info = self.user_cooldown_info_cache(
			user_id,
			connection,
			&cache_lock,
		).await?; // TODO: maybe cooldown err instead

		self.connections.set_user_cooldown(user_id, cooldown_info.clone()).await;

		let stats = statistics_lock.clone();
		self.connections.send(socket::Packet::BoardStatsUpdated { stats }).await;

		Ok((changes, timestamp))
	}

	pub async fn try_undo(
		&self,
		user_id: &str,
		position: u64,
		connection: &BoardsConnection,
	) -> Result<CooldownInfo, UndoError> {
		if connection.is_user_banned(user_id).await? {
			return Err(UndoError::Banned)
		}

		let uid = connection.get_uid(user_id).await?;
		
		// FIXME: use this to remove pixel from cache
		let mut cache = self.placement_cache.write().await;

		let (sector_index, sector_offset) = self.info.shape
			.to_local(position as usize)
			.ok_or(UndoError::OutOfBounds)?;

		let mut sector = self.sectors
			.get_sector_mut(sector_index, connection).await?
			.expect("Missing sector");

		// This acts as a per-user lock to prevent exploits bypassing cooldown
		// NOTE: no longer needed as placement_cache eclipses it and is a global lock
		let mut statistics_lock = self.statistics_cache.lock(uid).await;

		let transaction = connection.begin().await?;
		
		let (undone_placement, last_placement) = transaction
			.get_two_placements(self.id, position).await?;

		let uid = connection.get_uid(user_id).await?;

		let placement_id = match undone_placement {
			Some(placement) if placement.user_id == uid => {
				let deadline = placement.modified + CONFIG.undo_deadline_seconds;
				if deadline > self.current_timestamp() {
					return Err(UndoError::Expired)
				}
				placement.id
			}
			_ => return Err(UndoError::WrongUser)
		};

		transaction.delete_placement(placement_id).await?;

		let (color, timestamp) = match last_placement {
			Some(placement) => (placement.color, placement.modified),
			None => (sector.initial[sector_offset], 0),
		};

		sector.colors[sector_offset] = color;
		sector.timestamps[(sector_offset * 4)..((sector_offset + 1) * 4)]
			.as_mut()
			.put_u32_le(timestamp);

		transaction.commit().await?;

		let mut activity_cache = self.activity_cache.lock().await;
		activity_cache.remove(timestamp, uid);

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

		self.connections.queue_board_change(data).await;

		let cooldown_info = self.user_cooldown_info_cache(
			user_id,
			connection,
			&cache,
		).await?;

		self.connections.set_user_cooldown(user_id, cooldown_info.clone()).await;

		let stats = statistics_lock.clone();
		self.connections.send(socket::Packet::BoardStatsUpdated { stats }).await;
		
		Ok(cooldown_info)
	}

	// TODO: re-evaluate anonymous placing, maybe try and implement it again
	pub async fn try_place(
		&self,
		user_id: &str,
		position: u64,
		color: u8,
		overrides: Overrides,
		connection: &BoardsConnection,
		users_connection: &mut UsersConnection,
	) -> Result<(CooldownInfo, Placement), PlaceError> {
		if connection.is_user_banned(user_id).await? {
			return Err(PlaceError::Banned)
		}

		let uid = connection.get_uid(user_id).await?;

		let (sector_index, sector_offset) = self.info.shape
			.to_local(position as usize)
			.ok_or(PlaceError::OutOfBounds)?;

		self.check_placement_palette(color, overrides.color)?;
		
		let mut cache = self.placement_cache.write().await;
		
		let sector = self.sectors
			.get_sector_mut(sector_index, connection).await?
			.expect("Missing sector");

		let mut sectors = HashMap::from([(sector_index, sector)]);
		let sector = sectors.get_mut(&sector_index).unwrap();
		if !overrides.mask {
			match MaskValue::try_from(sector.mask[sector_offset]).ok() {
				Some(MaskValue::Place) => Ok(()),
				Some(MaskValue::NoPlace) => Err(PlaceError::Unplacable),
				// NOTE: there exists an old implementation in the version
				// control history. It's messy and would need to load adjacent
				// sectors now so I'm dropping it for now.
				Some(MaskValue::Adjacent) => unimplemented!(),
				None => Err(PlaceError::UnknownMaskValue),
			}?;
		}

		if sector.colors[sector_offset] == color {
			return Err(PlaceError::NoOp);
		}

		// This acts as a per-user lock to prevent exploits bypassing cooldown
		// NOTE: no longer needed as placement_cache eclipses it and is a global lock
		let mut statistics_lock = self.statistics_cache.lock(uid).await;

		let timestamp = self.current_timestamp();
		// TODO: ignore cooldown should probably also mark the pixel as not
		// contributing to the pixels available
		if !overrides.cooldown {
			let cooldown_info = self.user_cooldown_info_cache(
				user_id,
				connection,
				&cache,
			).await?;

			if cooldown_info.pixels_available == 0 {
				return Err(PlaceError::Cooldown);
			}
		}

		let new_placement = connection.insert_placement(
			self.id,
			position,
			color,
			timestamp,
			user_id,
			users_connection,
		).await?;

		cache.insert(CachedPlacement {
			position: new_placement.position,
			modified: new_placement.modified,
			user_id: uid,
		});

		let mut activity_cache = self.activity_cache.lock().await;
		activity_cache.insert(new_placement.modified, uid);

		let sector = sectors.get_mut(&sector_index).unwrap();

		sector.colors[sector_offset] = color;
		sector.timestamps[(sector_offset * 4)..((sector_offset + 1) * 4)]
			.as_mut()
			.put_u32_le(timestamp);

		statistics_lock.colors.entry(color).or_default().placed += 1;

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

		self.connections.queue_board_change(data).await;

		let cooldown_info = self.user_cooldown_info_cache(
			user_id,
			connection,
			&cache,
		).await?; // TODO: maybe cooldown err instead

		self.connections.set_user_cooldown(user_id, cooldown_info.clone()).await;

		let stats = statistics_lock.clone();
		self.connections.send(socket::Packet::BoardStatsUpdated { stats }).await;

		Ok((cooldown_info, new_placement))
	}

	pub async fn list_placements(
		&self,
		token: PlacementPageToken,
		limit: usize,
		order: Order,
		filter: PlacementFilter,
		connection: &BoardsConnection,
		users_connection: &mut UsersConnection,
	) -> Result<Page<Placement>, BoardsDatabaseError> {
		connection.list_placements(
			self.id,
			token,
			limit,
			order,
			filter,
			users_connection,
		).await
	}

	pub async fn lookup(
		&self,
		position: u64,
		connection: &BoardsConnection,
		users_connection: &mut UsersConnection,
	) -> Result<Option<Placement>, BoardsDatabaseError> {
		connection.get_placement(self.id, position, users_connection).await
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

	async fn calculate_cooldowns<const T: usize>(
		&self,
		placement: Option<&CachedPlacement>,
		connection: &BoardsConnection,
		cache: &PlacementCache<T>,
	) -> Result<Vec<SystemTime>, BoardsDatabaseError> {
		let parameters = if let Some(placement) = placement {
			let timestamp = placement.modified;

			let activity = self.user_count_for_time(
				timestamp,
				connection,
				cache,
			).await?;

			let density = self.density_for_time(
				timestamp,
				placement.position,
				connection,
			).await?;
			
			CooldownParameters { activity, density, timestamp }
		} else {
			CooldownParameters::default()
		};

		let CooldownParameters { activity, density, timestamp } = parameters;

		let base_time = self.info.created_at + timestamp as u64;
		let base_time = Duration::from_secs(base_time);
		let max_pixels = self.info.max_pixels_available;
		let max_pixels = usize::try_from(max_pixels).unwrap();

		// TODO: proper cooldown
		Ok(std::iter::repeat(Duration::from_secs(CONFIG.cooldown))
			.take(max_pixels)
			.enumerate()
			.map(|(i, c)| c * (i + 1) as u32)
			.map(|cooldown| UNIX_EPOCH + base_time + cooldown)
			.collect())
	}

	pub async fn user_cooldown_info(
		&self,
		user_id: &str,
		connection: &BoardsConnection,
	) -> Result<CooldownInfo, BoardsDatabaseError> {
		let placement_cache = self.placement_cache.read().await;
		self.user_cooldown_info_cache(
			user_id,
			connection,
			&placement_cache,
		).await
	}

	// TODO: If any code here is a mess, this certainly is.
	// The explanations don't even make sense: just make it readable.
	async fn user_cooldown_info_cache<const T: usize>(
		&self,
		user_id: &str,
		connection: &BoardsConnection,
		placement_cache: &PlacementCache<T>,
	) -> Result<CooldownInfo, BoardsDatabaseError> {
		let max_pixels = self.info.max_pixels_available as usize;
		let uid = connection.get_uid(user_id).await?;
		let mut placements = placement_cache.iter()
			.filter(|p| p.user_id == uid)
			.take(max_pixels)
			.cloned()
			.collect::<Vec<_>>();
		placements.reverse();

		if placements.len() < max_pixels {
			// if we don't have all the user's placements in the buffer,
			// we have to query the database as we don't know if we're missing
			// placements.
			placements = connection.list_user_placements(
				self.id,
				user_id,
				max_pixels,
			).await?;
		};

		let cooldowns = self.calculate_cooldowns(
			placements.last(),
			connection,
			placement_cache,
		).await?;

		let mut info = CooldownInfo::new(cooldowns, SystemTime::now());

		// If we would already have MAX_STACKED just from waiting, we
		// don't need to check previous data since we can't possibly
		// have more.
		// Similarly, we know we needed to spend a pixel on the most
		// recent placement so we can't have saved more than
		// `MAX_STACKED - 1` since then.
		// TODO: actually, I think this generalizes and we only have to
		// check the last `Board::MAX_STACKED - current_stacked` pixels.
		let max_minus_one = self.info.max_pixels_available.saturating_sub(1) as usize;
		let incomplete_info_is_correct = info.pixels_available >= max_minus_one;

		if !placements.is_empty() && !incomplete_info_is_correct {
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

			let mut pixels: usize = 0;

			for pair in placements.windows(2) {
				let info = CooldownInfo::new(
					self.calculate_cooldowns(
						Some(&pair[0]),
						connection,
						placement_cache,
					).await?,
					UNIX_EPOCH
						+ Duration::from_secs(
							u64::from(pair[1].modified) + self.info.created_at,
						),
				);

				pixels = pixels
					.max(info.pixels_available)
					.saturating_sub(1);
			}

			info.pixels_available = info.pixels_available.max(pixels);
		}

		Ok(info)
	}

	async fn user_count_for_time<const T: usize>(
		&self,
		timestamp: u32,
		connection: &BoardsConnection,
		cache: &PlacementCache<T>,
	) -> Result<usize, BoardsDatabaseError> {
		let idle_timeout = self.idle_timeout();
		let max_time = i32::try_from(timestamp).unwrap();
		let min_time = i32::try_from(timestamp.saturating_sub(idle_timeout)).unwrap();

		let cache_age = cache.iter().last().unwrap().modified as i32;
		if min_time > cache_age {
			let user_count = cache.iter()
				// TODO: binary search
				.skip_while(|p| p.modified > max_time as u32)
				.take_while(|p| p.modified >= min_time as u32)
				.map(|p| p.user_id)
				.collect::<HashSet<_>>().len();

			Ok(user_count)
		} else {
			connection.user_count_between(self.id, min_time, max_time).await
		}
	}

	async fn density_for_time(
		&self,
		timestamp: u32,
		position: u64,
		connection: &BoardsConnection,
	) -> Result<u32, BoardsDatabaseError> {
		connection.density_for_time(
			self.id,
			position as i64,
			timestamp as i32,
		).await
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
		&mut self,
		socket: &Arc<Socket>,
		connection: &BoardsConnection,
	) -> Result<(), BoardsDatabaseError> {
		let id = socket.user_id().await;

		// TODO: is this needed?
		let cooldown_info = if let Some(user_id) = id {
			Some(self.user_cooldown_info(&user_id, connection).await?)
		} else {
			None
		};

		self.connections.insert(socket, cooldown_info).await;

		Ok(())
	}

	pub async fn remove_socket(
		&mut self,
		socket: &Arc<Socket>,
	) {
		self.connections.remove(socket).await
	}

	pub async fn create_notice(
		&self,
		title: String,
		content: String,
		expiry: Option<u64>,
		connection: &BoardsConnection,
		users_connection: &mut UsersConnection,
	) -> Result<Reference<BoardsNotice>, DatabaseError> {
		let notice = connection.create_board_notice(
			self.id,
			title,
			content,
			expiry,
			users_connection,
		).await?;

		let packet = Packet::BoardNoticeCreated {
			notice: notice.clone(),
		};

		self.connections.send(packet).await;

		Ok(notice)
	}

	pub async fn edit_notice(
		&self,
		id: usize,
		title: Option<String>,
		content: Option<String>,
		expiry: Option<Option<u64>>,
		connection: &BoardsConnection,
		users_connection: &mut UsersConnection,
	) -> Result<Reference<BoardsNotice>, DatabaseError> {
		let notice = connection.edit_board_notice(
			self.id,
			id,
			title,
			content,
			expiry,
			users_connection,
		).await?;

		let packet = Packet::BoardNoticeUpdated {
			notice: notice.clone(),
		};

		self.connections.send(packet).await;

		Ok(notice)
	}

	pub async fn delete_notice(
		&self,
		id: usize,
		connection: &BoardsConnection,
	) -> Result<bool, BoardsDatabaseError> {
		let notice = connection.delete_board_notice(self.id, id).await?;

		let packet = Packet::BoardNoticeDeleted {
			notice: BoardsNotice::uri(self.id, id as _),
		};

		self.connections.send(packet).await;

		Ok(notice)
	}

	pub async fn user_stats(
		&self,
		user: &str,
		connection: &BoardsConnection,
	) -> Result<PlacementColorStatistics, DatabaseError> {
		let uid = connection.get_uid(user).await?;
		Ok((*self.statistics_cache.lock(uid).await).clone())
	}
}
