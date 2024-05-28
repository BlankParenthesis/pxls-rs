mod socket;
mod cooldown;
mod color;
mod sector;
mod shape;
mod info;
mod placement;

use std::{
	convert::TryFrom,
	io::{Seek, SeekFrom},
	sync::Arc,
	time::{Duration, SystemTime, UNIX_EPOCH}, collections::{HashMap, hash_map::Entry, HashSet},
};

use bytes::BufMut;
use serde::Serialize;
use tokio::sync::RwLock;
use warp::http::{StatusCode, Uri};
use warp::{reject::Reject, reply::Response, Reply};

use crate::routes::{board_moderation::boards::pixels::Overrides, board_notices::boards::notices::PreparedBoardsNotice, core::boards::pixels::PlacementFilter};
use crate::config::CONFIG;
use crate::database::{UsersConnection, DatabaseError};
use crate::filter::response::{paginated_list::Page, reference::{Referenceable, Reference}};
use crate::filter::body::patch::BinaryPatch;
use crate::database::BoardsDatabaseError;
use crate::AsyncWrite;
use crate::database::{BoardsConnection, Order};

use socket::{Connections, Packet, Socket};
use sector::{SectorAccessor, SectorCache, MaskValue, IoError};
use cooldown::CooldownInfo;
use info::BoardInfo;

pub use color::{Color, Palette};
pub use sector::{SectorBuffer, Sector};
pub use shape::Shape;
pub use placement::{Placement, PlacementPageToken, CachedPlacement};
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

struct PlacementCache<const SIZE: usize> {
	ring_buffer: Box<[CachedPlacement; SIZE]>,
	// assumes infinite size, wrapped at lookup
	position: usize,
}

impl<const SIZE: usize> PlacementCache<SIZE> {
	pub async fn fill(board_id: i32, connection: &BoardsConnection) -> Result<Self, BoardsDatabaseError> {
		let mut placements = connection.list_placements_simple(
			board_id,
			Order::Reverse,
			SIZE,
		).await?;

		placements.reverse();
		let position = placements.len();

		let mut ring_buffer: Box<[_; SIZE]> = vec![CachedPlacement::null(); SIZE]
			.into_boxed_slice()
			.try_into().unwrap();
		
		ring_buffer.copy_from_slice(&placements);

		Ok(Self { ring_buffer, position })
	}

	pub fn iter(&self) -> PlacementCacheIterator<'_, SIZE> {
		PlacementCacheIterator {
			placements: self,
			offset: 0,
		}
	}

	fn insert(&mut self, placement: CachedPlacement) {
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
		if self.size_hint().0 > 0 {
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

#[derive(Debug, Default, Clone, Copy)]
struct CooldownParameters {
	activity: usize,
	density: usize,
	timestamp: u32,
}

pub struct Board {
	pub id: i32,
	pub info: BoardInfo,
	connections: Connections,
	sectors: SectorCache,
	placement_cache: RwLock<Option<PlacementCache<PLACEMENT_CACHE_SIZE>>>,
}

impl From<&Board> for Uri {
	fn from(board: &Board) -> Self {
		format!("/boards/{}", board.id)
			.parse::<Uri>()
			.unwrap()
	}
}

impl Referenceable for Board {
	fn location(&self) -> Uri { Uri::from(self) }
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
			placement_cache: RwLock::new(None),
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
		
		self.connections.send_board_update(packet).await;

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

		self.connections.send(packet).await;

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
			Some(_) => Ok(()),
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

		// lock all the relevant sectors
		let mut sectors = HashMap::new();
		for sector_index in used_sectors {
			if let Entry::Vacant(vacant) = sectors.entry(sector_index) {
				let sector =  self.sectors
					.get_sector_mut(sector_index, connection).await
					.map_err(PlaceError::DatabaseError)?
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

		if !overrides.cooldown {
			let cooldown_info = self.user_cooldown_info(
				user_id,
				connection,
			).await.map_err(PlaceError::DatabaseError)?;

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
			user_id.to_owned(),
		).await.map_err(PlaceError::DatabaseError)?;
		
		for ((sector_index, sector_offset), color) in sector_placements {
			let sector = sectors.get_mut(&sector_index).unwrap();

			sector.colors[sector_offset] = color;
			
			let timestamp_slice =
				&mut sector.timestamps[(sector_offset * 4)..((sector_offset + 1) * 4)];
			
			timestamp_slice
				.as_mut()
				.put_u32_le(timestamp);
		}

		Ok((changes, timestamp))
	}

	pub async fn try_undo(
		&self,
		user_id: &str,
		position: u64,
		connection: &BoardsConnection,
	) -> Result<(), UndoError> {
		let (sector_index, sector_offset) = self.info.shape
			.to_local(position as usize)
			.ok_or(UndoError::OutOfBounds)?;

		let mut sector = self.sectors
			.get_sector_mut(sector_index, connection).await
			.map_err(UndoError::DatabaseError)?
			.expect("Missing sector");

		let transaction = connection.begin().await
			.map_err(UndoError::DatabaseError)?;
		
		let (undone_placement, last_placement) = transaction
			.get_two_placements(self.id, position).await
			.map_err(UndoError::DatabaseError)?;

		let placement_id = match undone_placement {
			Some(Placement { id, user, timestamp, .. }) if user == user_id => {
				let deadline = timestamp + CONFIG.undo_deadline_seconds;
				if deadline > self.current_timestamp() {
					return Err(UndoError::Expired)
				}
				id
			}
			_ => return Err(UndoError::WrongUser)
		};

		transaction.delete_placement(placement_id).await
			.map_err(UndoError::DatabaseError)?;

		let (color, timestamp) = match last_placement {
			Some(placement) => (placement.color, placement.timestamp),
			None => (sector.initial[sector_offset], 0),
		};

		sector.colors[sector_offset] = color;
		sector.timestamps[(sector_offset * 4)..((sector_offset + 1) * 4)]
			.as_mut()
			.put_u32_le(timestamp);

		transaction.commit().await
			.map_err(UndoError::DatabaseError)?;

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

		self.connections.send_board_update(data).await;

		let cooldown_info = self.user_cooldown_info(
			user_id,
			connection,
		).await.map_err(UndoError::DatabaseError)?;

		self.connections.set_user_cooldown(user_id, cooldown_info).await;
		
		Ok(())
	}

	// TODO: re-evaluate anonymous placing, maybe try and implement it again
	pub async fn try_place(
		&self,
		user_id: &str,
		position: u64,
		color: u8,
		overrides: Overrides,
		connection: &BoardsConnection,
	) -> Result<Placement, PlaceError> {
		// TODO: I hate most things about how this is written.
		// Redo it and/or move stuff.

		let (sector_index, sector_offset) = self.info.shape
			.to_local(position as usize)
			.ok_or(PlaceError::OutOfBounds)?;

		self.check_placement_palette(color, overrides.color)?;
		
		let mut sector = self.sectors
			.get_sector_mut(sector_index, connection).await
			.map_err(PlaceError::DatabaseError)?
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

		let timestamp = self.current_timestamp();
		// TODO: ignore cooldown should probably also mark the pixel as not
		// contributing to the pixels available
		if !overrides.cooldown {
			let cooldown_info = self.user_cooldown_info(
				user_id,
				connection,
			).await.map_err(PlaceError::DatabaseError)?;

			if cooldown_info.pixels_available == 0 {
				return Err(PlaceError::Cooldown);
			}
		}

		// FIXME: the sector write guard prevents double writes to this sector,
		// but not across multiple sectors, so a user could place twice at once
		// in two different sectors.
		// Could probably solve this with sql transactions and select's `lock`
		let new_placement = connection.insert_placement(
			self.id,
			position,
			color,
			timestamp,
			user_id.to_owned(),
		).await.map_err(PlaceError::DatabaseError)?;

		let mut cache_lock = self.placement_cache.write().await;
		let cache = cache_lock.as_mut().unwrap();
		cache.insert(CachedPlacement::from(&new_placement));
		drop(cache_lock);

		sector.colors[sector_offset] = color;
		sector.timestamps[(sector_offset * 4)..((sector_offset + 1) * 4)]
			.as_mut()
			.put_u32_le(timestamp);

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

		self.connections.send_board_update(data).await;

		let cooldown_info = self.user_cooldown_info(
			user_id,
			connection,
		).await.map_err(PlaceError::DatabaseError)?;

		self.connections.set_user_cooldown(user_id, cooldown_info).await;

		Ok(new_placement)
	}

	pub async fn list_placements(
		&self,
		token: PlacementPageToken,
		limit: usize,
		order: Order,
		filter: PlacementFilter,
		connection: &BoardsConnection,
	) -> Result<Page<Placement>, BoardsDatabaseError> {
		connection.list_placements(self.id, token, limit, order, filter).await
	}

	pub async fn lookup(
		&self,
		position: u64,
		connection: &BoardsConnection,
	) -> Result<Option<Placement>, BoardsDatabaseError> {
		connection.get_placement(self.id, position).await
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

	async fn calculate_cooldowns(
		&self,
		placement: Option<&CachedPlacement>,
		connection: &BoardsConnection,
	) -> Result<Vec<SystemTime>, BoardsDatabaseError> {
		let parameters = if let Some(placement) = placement {
			let activity = self.user_count_for_time(
				placement.timestamp,
				connection
			).await?;

			let density = connection.count_placements(
				self.id,
				placement.position,
				placement.timestamp,
			).await?;

			let timestamp = placement.timestamp;
			
			CooldownParameters { activity, density, timestamp }
		} else {
			CooldownParameters::default()
		};

		let CooldownParameters { activity, density, timestamp } = parameters;

		let base_time = self.info.created_at + timestamp as u64;
		let base_time = Duration::from_secs(base_time);
		let max_pixels = self.info.max_pixels_available;
		let max_pixels = usize::try_from(max_pixels).unwrap();

		const COOLDOWN: Duration = Duration::from_secs(30);

		// TODO: proper cooldown
		Ok(std::iter::repeat(COOLDOWN)
			.take(max_pixels)
			.enumerate()
			.map(|(i, c)| c * (i + 1) as u32)
			.map(|cooldown| UNIX_EPOCH + base_time + cooldown)
			.collect())
	}

	// TODO: If any code here is a mess, this certainly is.
	// The explanations don't even make sense: just make it readable.
	pub async fn user_cooldown_info(
		&self,
		user_id: &str,
		connection: &BoardsConnection,
	) -> Result<CooldownInfo, BoardsDatabaseError> {
		let max_pixels = self.info.max_pixels_available as usize;
		let cache = self.placement_cache.read().await;
		let uid = connection.get_uid(user_id).await?;
		let mut placements = match cache.as_ref() {
			Some(cache) => {
				cache.iter()
					.filter(|p| p.user_id == uid)
					.take(max_pixels)
					.cloned()
					.collect()
			},
			None => {
				vec![]
			},
		};
		drop(cache);

		if placements.len() < max_pixels {
			// if we don't have all the user's placements in the buffer,
			// we have to query the database as we don't know if we're missing
			// placements.
			placements = connection.list_user_placements(
				self.id,
				user_id,
				max_pixels,
			).await?.into_iter().map(CachedPlacement::from).collect();
		};

		let cooldowns = self.calculate_cooldowns(placements.last(), connection).await?;
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
					self.calculate_cooldowns(Some(&pair[0]), connection).await?,
					UNIX_EPOCH
						+ Duration::from_secs(
							u64::from(pair[1].timestamp) + self.info.created_at,
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

	async fn cached_placements(
		&self,
		connection: &BoardsConnection,
	) -> Result<tokio::sync::RwLockReadGuard<'_, Option<PlacementCache<PLACEMENT_CACHE_SIZE>>>, BoardsDatabaseError> {

		let cache_lock = self.placement_cache.read().await;

		if cache_lock.is_none() {
			drop(cache_lock);

			let mut cache = self.placement_cache.write().await;
			if cache.is_none() {
				let new_cache = PlacementCache::fill(self.id, connection).await?;
				cache.replace(new_cache);
			}
			drop(cache);

			Ok(self.placement_cache.read().await)
		} else {
			Ok(cache_lock)
		}
	}

	async fn user_count_for_time(
		&self,
		timestamp: u32,
		connection: &BoardsConnection,
	) -> Result<usize, BoardsDatabaseError> {
		let idle_timeout = self.idle_timeout();
		let max_time = i32::try_from(timestamp).unwrap();
		let min_time = i32::try_from(timestamp.saturating_sub(idle_timeout)).unwrap();

		let cache = self.cached_placements(connection).await?;
		let cache = cache.as_ref().unwrap();
		let cache_age = cache.iter().last().unwrap().timestamp as i32;
		if min_time > cache_age {
			let user_count = cache.iter()
				.skip_while(|p| p.timestamp > max_time as u32)
				.take_while(|p| p.timestamp >= min_time as u32)
				.map(|p| p.user_id)
				.collect::<HashSet<_>>().len();

			Ok(user_count)
		} else {
			connection.user_count_between(self.id, min_time, max_time).await
		}
	}

	pub async fn user_count(
		&self,
		connection: &BoardsConnection,
	) -> Result<usize, BoardsDatabaseError> {
		self.user_count_for_time(self.current_timestamp(), connection).await
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

		let cooldown_info = if let Some(ref user_id) = id {
			Some(self.user_cooldown_info(user_id, connection).await?)
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
	) -> Result<PreparedBoardsNotice, DatabaseError> {
		let notice = connection.create_board_notice(
			self.id,
			title,
			content,
			expiry,
		).await?
			.prepare(users_connection).await?;

		let packet = Packet::BoardNoticeCreated {
			notice: Reference::from(&notice),
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
	) -> Result<PreparedBoardsNotice, DatabaseError> {
		let notice = connection.edit_board_notice(
			self.id,
			id,
			title,
			content,
			expiry,
		).await?
			.prepare(users_connection).await?;

		let packet = Packet::BoardNoticeUpdated {
			notice: Reference::from(&notice),
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
			notice: format!("/boards/{}/notices/{}", self.id, id).parse().unwrap(),
		};

		self.connections.send(packet).await;

		Ok(notice)
	}
}