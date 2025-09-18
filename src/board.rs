mod socket;
mod cooldown;
mod color;
mod sector;
mod shape;
mod info;
mod placement;
mod activity;

use std::{
	collections::{hash_map::Entry, HashMap, HashSet},
	convert::TryFrom,
	io::{Seek, SeekFrom},
	sync::Arc,
	time::{SystemTime, UNIX_EPOCH},
};

use bytes::BufMut;
use serde::Serialize;
use tokio::sync::{mpsc::{self, error::SendError}, Mutex, RwLock};
use tokio::time::{Duration, Instant};
use warp::http::{StatusCode, Uri};
use warp::{reject::Reject, reply::Response, Reply};

use crate::routes::board_moderation::boards::pixels::Overrides;
use crate::routes::placement_statistics::users::PlacementColorStatistics;
use crate::routes::board_notices::boards::notices::BoardsNotice;
use crate::routes::core::boards::pixels::PlacementFilter;
use crate::config::CONFIG;
use crate::database::{BoardsDatabase, Database, User, UsersConnection, DatabaseError};
use crate::filter::response::{paginated_list::Page, reference::Reference};
use crate::filter::body::patch::BinaryPatch;
use crate::filter::header::range::Range;
use crate::database::BoardsDatabaseError;
use crate::AsyncWrite;
use crate::database::{BoardsConnection, Order};

use socket::{Connections, Packet, Socket};
use sector::{SectorAccessor, BufferedSectorCache, MaskValue, IoError, BufferedSector};
use cooldown::CooldownInfo;
use info::BoardInfo;

pub use activity::ActivityCache;
pub use cooldown::CooldownCache;
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
	SenderError(SendError<PendingPlacement>),
	Banned,
}

impl From<BoardsDatabaseError> for PlaceError {
	fn from(value: BoardsDatabaseError) -> Self {
		Self::DatabaseError(value)
	}
}

impl From<SendError<PendingPlacement>> for PlaceError {
	fn from(value: SendError<PendingPlacement>) -> Self {
		Self::SenderError(value)
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
			Self::DatabaseError(_) | Self::SenderError(_) => {
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
	pub uid: i32,
}

pub struct Board {
	pub id: i32,
	pub info: BoardInfo,
	connections: RwLock<Connections>,
	sectors: BufferedSectorCache,
	statistics_cache: StatisticsHashLock<i32>,
	activity_cache: Mutex<ActivityCache>,
	cooldown_cache: RwLock<CooldownCache>,
	placement_sender: mpsc::Sender<PendingPlacement>,
	pool: Arc<BoardsDatabase>,
	lookup_cache: Mutex<HashMap<u64, Option<Placement>>>,
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
		activity_cache: Mutex<ActivityCache>,
		cooldown_cache: RwLock<CooldownCache>,
		pool: Arc<BoardsDatabase>,
	) -> Self {
		let info = BoardInfo {
			name,
			created_at,
			shape,
			palette,
			max_pixels_available,
		};

		let sectors = BufferedSectorCache::new(
			id,
			info.shape.sector_count(),
			info.shape.sector_size(),
			pool.clone(),
		);

		let connections = RwLock::new(Connections::default());
		
		let (placement_sender, placement_receiver) = mpsc::channel(10000);

		tokio::spawn(Self::buffer_placements(id, placement_receiver, pool.clone()));
		
		let lookup_cache = Mutex::new(HashMap::new());

		Self {
			id,
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
		board_id: i32,
		mut receiver: mpsc::Receiver<PendingPlacement>,
		pool: Arc<BoardsDatabase>,
	) {
		let mut buffer = vec![];
		let tick_time = Duration::from_millis(
			CONFIG.database_tickrate.map(|r| (1000.0 / r) as u64).unwrap_or(0)
		);
		let mut next_tick = Instant::now() + tick_time;
		while receiver.recv_many(&mut buffer, 10000).await > 0 {
			let connection = pool.connection().await
				.expect("A board database insert thread failed to connect to the database");
			connection.insert_placements(board_id, buffer.drain(..).as_slice()).await
				.expect("A board database insert thread failed to insert placements");
			
			tokio::time::sleep_until(next_tick).await;
			next_tick = Instant::now() + tick_time;
		}
	}

	pub async fn read<'l>(
		&'l self,
		buffer: SectorBuffer,
		connection: &'l BoardsConnection,
	) -> SectorAccessor<'l> {
		self.sectors.cache.access(buffer, connection)
	}
	
	pub async fn try_read_exact_sector(
		&self,
		range: Range,
		sector_type: SectorBuffer,
		timestamps: bool,
	) -> Option<(Arc<Result<Option<BufferedSector>, BoardsDatabaseError>>, std::ops::Range<usize>)> {
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
		connection: &BoardsConnection,
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

			self.sectors = BufferedSectorCache::new(
				self.id,
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
			let cooldown = self.user_cooldown_info(user, connection).await?;
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
		connection: &BoardsConnection,
	) -> Result<(), BoardsDatabaseError> {
		let mut connections = self.connections.write().await;
		connections.close();
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
	
	async fn insert_placement(
		&self,
		user_id: &str,
		position: u64,
		color: u8,
		timestamp: u32,
		sector_offset: usize,
		sector: &mut Sector,
		connection: &BoardsConnection,
		users_connection: &mut UsersConnection,
	) -> Result<Placement, PlaceError> {
		let uid = connection.get_uid(user_id).await?;
		
		let four_byte_range = (sector_offset * 4)..((sector_offset + 1) * 4);
		
		let mut cooldown_cache = self.cooldown_cache.write().await;
		let mut activity_cache = self.activity_cache.lock().await;
		
		let pending_placement = PendingPlacement {
			uid,
			timestamp,
			position,
			color,
		};
		self.placement_sender.send(pending_placement).await?;
		
		if let Ok(user) = users_connection.get_user(user_id).await {
			let placement = Placement {
				position,
				color,
				modified: timestamp,
				user: Reference::new(User::uri(&user_id), user.clone()),
			};
			let mut lookup_cache = self.lookup_cache.lock().await;
			lookup_cache.insert(position, Some(placement));
			drop(lookup_cache);
		}
		
		let user = users_connection.get_user(user_id).await
			.map_err(BoardsDatabaseError::UsersError)?;
		
		let new_placement = Placement {
			position: position as u64,
			color: color as u8,
			modified: timestamp as u32,
			user: Reference::new(User::uri(user_id), user.clone()),
		};
		
		activity_cache.insert(new_placement.modified, uid);
		
		let activity = activity_cache.count(new_placement.modified) as u32;
		let density = u32::from_le_bytes(
			sector.density[four_byte_range.clone()].try_into().unwrap()
		);
		
		cooldown_cache.insert(new_placement.modified, uid, activity, density);
		
		sector.colors[sector_offset] = color;
		sector.timestamps[four_byte_range]
			.as_mut()
			.put_u32_le(timestamp);
		
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
		user_id: &str,
		placements: &[(u64, u8)],
		overrides: Overrides,
		connection: &BoardsConnection,
		users_connection: &mut UsersConnection,
	) -> Result<(usize, u32), PlaceError> {
		let uid = connection.get_uid(user_id).await?;
		
		if connection.is_user_banned(user_id).await? {
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
		let mut statistics_lock = self.statistics_cache.lock(uid).await;

		if !overrides.cooldown {
			let cooldown_info = self.user_cooldown_info(
				user_id,
				connection,
			).await?;

			if (cooldown_info.pixels_available as usize) < changes {
				return Err(PlaceError::Cooldown);
			}
		}

		let timestamp = self.current_timestamp();
		
		for (position, (sector_index, sector_offset), color) in sector_placements {
			let sector = sectors.get_mut(&sector_index).unwrap();

			self.insert_placement(
				user_id,
				position,
				color,
				timestamp,
				sector_offset,
				sector,
				connection,
				users_connection,
			).await?;
			
			statistics_lock.colors.entry(color).or_default().placed += 1;
		}

		let cooldown_info = self.user_cooldown_info(
			user_id,
			connection,
		).await?; // TODO: maybe cooldown err instead

		let connections = self.connections.read().await;
		connections.set_user_cooldown(user_id, cooldown_info.clone()).await;

		let stats = statistics_lock.clone();
		connections.send(socket::Packet::BoardStatsUpdated { stats }).await;

		Ok((changes, timestamp))
	}

	pub async fn try_undo(
		&self,
		user_id: &str,
		position: u64,
		connection: &BoardsConnection,
	) -> Result<CooldownInfo, UndoError> {
		let uid = connection.get_uid(user_id).await?;
		
		if connection.is_user_banned(user_id).await? {
			return Err(UndoError::Banned);
		}
		
		let (sector_index, sector_offset) = self.info.shape
			.to_local(position as usize)
			.ok_or(UndoError::OutOfBounds)?;

		let mut sector = self.sectors.cache
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
				if deadline < self.current_timestamp() {
					return Err(UndoError::Expired)
				}
				placement.id
			}
			_ => return Err(UndoError::WrongUser)
		};

		transaction.delete_placement(placement_id).await?;
		let mut lookup_cache = self.lookup_cache.lock().await;
		lookup_cache.remove(&position);
		drop(lookup_cache);

		let (color, timestamp) = match last_placement {
			Some(placement) => (placement.color, placement.modified),
			None => (sector.initial[sector_offset], 0),
		};

		sector.colors[sector_offset] = color;
		sector.timestamps[(sector_offset * 4)..((sector_offset + 1) * 4)]
			.as_mut()
			.put_u32_le(timestamp);

		transaction.commit().await?;

		let mut cooldown_cache = self.cooldown_cache.write().await;
		let mut activity_cache = self.activity_cache.lock().await;
		activity_cache.remove(timestamp, uid);
		cooldown_cache.remove(timestamp, uid);
		drop(cooldown_cache);

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

		let cooldown_info = self.user_cooldown_info(
			user_id,
			connection,
		).await?;
		
		let connections = self.connections.read().await;
		connections.queue_board_change(data).await;
		connections.set_user_cooldown(user_id, cooldown_info.clone()).await;

		let stats = statistics_lock.clone();
		connections.send(socket::Packet::BoardStatsUpdated { stats }).await;
		
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
		let uid = connection.get_uid(user_id).await?;
		
		if connection.is_user_banned(user_id).await? {
			return Err(PlaceError::Banned);
		}

		let (sector_index, sector_offset) = self.info.shape
			.to_local(position as usize)
			.ok_or(PlaceError::OutOfBounds)?;

		self.check_placement_palette(color, overrides.color)?;
		
		let mut sector = self.sectors.cache
			.get_sector_mut(sector_index, connection).await?
			.expect("Missing sector");

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
			let cooldown_info = self.user_cooldown_info(
				user_id,
				connection,
			).await?;

			if cooldown_info.pixels_available == 0 {
				return Err(PlaceError::Cooldown);
			}
		}

		let new_placement = self.insert_placement(
			user_id,
			position,
			color,
			timestamp,
			sector_offset,
			&mut sector,
			connection,
			users_connection,
		).await?;
		drop(sector);

		statistics_lock.colors.entry(color).or_default().placed += 1;

		// TODO: maybe cooldown err instead
		let cooldown_info = self.user_cooldown_info(user_id, connection).await?;
		let connections = self.connections.read().await;
		connections.set_user_cooldown(user_id, cooldown_info.clone()).await;

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
		let mut cache = self.lookup_cache.lock().await;
		if !cache.contains_key(&position) {
			let placement = connection.get_placement(self.id, position, users_connection).await?;
			cache.insert(position, placement);
		}
		
		Ok(cache.get(&position).unwrap().clone())
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
		user_id: &str,
		connection: &BoardsConnection,
	) -> Result<CooldownInfo, BoardsDatabaseError> {
		let uid = connection.get_uid(user_id).await?;
		let cache = self.cooldown_cache.read().await;
		Ok(cache.get(uid, self.current_timestamp()))
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
		connection: &BoardsConnection,
	) -> Result<(), BoardsDatabaseError> {
		let id = socket.user_id().await;
		// TODO: is this needed?
		let cooldown_info = if let Some(user_id) = id {
			Some(self.user_cooldown_info(&user_id, connection).await?)
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

		let connections = self.connections.read().await;
		connections.send(packet).await;

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

		let connections = self.connections.read().await;
		connections.send(packet).await;

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

		let connections = self.connections.read().await;
		connections.send(packet).await;

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
