use std::{
	collections::{HashMap, HashSet},
	convert::TryFrom,
	io::{Seek, SeekFrom},
	sync::{Arc, RwLock, Weak},
	time::{Duration, SystemTime, UNIX_EPOCH},
};

use bytes::BufMut;
use enum_map::EnumMap;
use http::{
	header::{HeaderName, HeaderValue},
	StatusCode, Uri,
};
use num_derive::FromPrimitive;
use num_traits::FromPrimitive;
use sea_orm::{ConnectionTrait, Set, ActiveValue::NotSet, EntityTrait, ColumnTrait, QueryFilter, QueryOrder, Order, QuerySelect, sea_query::Expr, ModelTrait, PaginatorTrait, TransactionTrait};
use serde::{Deserialize, Serialize};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use warp::{reject::Reject, reply::Response, Reply};

use crate::{
	filters::body::patch::BinaryPatch,
	objects::{
		packet, AuthedSocket, AuthedUser, Color, Extension, Palette, Reference, SectorBuffer,
		SectorCache, SectorCacheAccess, Shape, User, UserCount, VecShape, color::replace_palette,
	}, database::{DbResult, entities::*}, DatabaseError,
};

use super::sector_cache::AsyncWrite;

#[derive(Serialize, Debug)]
pub struct BoardInfo {
	name: String,
	created_at: u64,
	shape: VecShape,
	palette: Palette,
	max_pixels_available: u32,
}

#[derive(Deserialize, Debug)]
pub struct BoardInfoPost {
	name: String,
	shape: VecShape,
	palette: Palette,
	max_pixels_available: u32,
}

#[derive(Deserialize, Debug)]
pub struct BoardInfoPatch {
	name: Option<String>,
	shape: Option<VecShape>,
	palette: Option<Palette>,
	max_pixels_available: Option<u32>,
}

impl From<BoardInfoPatch> for packet::server::BoardInfo {
	fn from(
		BoardInfoPatch {
			name,
			shape,
			palette,
			max_pixels_available,
		}: BoardInfoPatch
	) -> Self {
		Self {
			name,
			shape,
			palette,
			max_pixels_available,
		}
	}
}

#[derive(Debug)]
struct UserConnections {
	connections: HashSet<Arc<AuthedSocket>>,
	cooldown_timer: Option<CancellationToken>,
}

impl UserConnections {
	fn new(
		socket: Arc<AuthedSocket>,
		cooldown_info: CooldownInfo,
	) -> Arc<RwLock<Self>> {
		let mut connections = HashSet::new();
		connections.insert(socket);

		let user_connections = Arc::new(RwLock::new(Self {
			connections,
			cooldown_timer: None,
		}));

		Self::set_cooldown_info(Arc::clone(&user_connections), cooldown_info);

		user_connections
	}

	fn insert(
		&mut self,
		socket: Arc<AuthedSocket>,
	) {
		self.connections.insert(socket);
	}

	fn remove(
		&mut self,
		socket: Arc<AuthedSocket>,
	) {
		self.connections.remove(&socket);
	}

	fn is_empty(&self) -> bool {
		self.connections.is_empty()
	}

	fn cleanup(&mut self) {
		assert!(self.is_empty());
		if let Some(timer) = self.cooldown_timer.take() {
			timer.cancel();
		}
	}

	fn set_cooldown_info(
		connections: Arc<RwLock<Self>>,
		cooldown_info: CooldownInfo,
	) {
		let weak = Arc::downgrade(&connections);
		let new_token = CancellationToken::new();

		let cloned_token = CancellationToken::clone(&new_token);

		let mut connections = connections.write().unwrap();

		if let Some(cancellable) = connections
			.cooldown_timer
			.replace(new_token)
		{
			cancellable.cancel();
		}

		let packet = packet::server::Packet::PixelsAvailable {
			count: u32::try_from(cooldown_info.pixels_available).unwrap(),
			next: cooldown_info
				.cooldown()
				.map(|timestamp| {
					timestamp
						.duration_since(UNIX_EPOCH)
						.unwrap()
						.as_secs()
				}),
		};

		connections.send(&packet);

		tokio::task::spawn(async move {
			tokio::select! {
				_ = cloned_token.cancelled() => (),
				_ = Self::cooldown_timer(weak, cooldown_info) => (),
			}
		});
	}

	fn send(
		&self,
		packet: &packet::server::Packet,
	) {
		let extension = Extension::from(packet);
		for connection in &self.connections {
			if connection
				.extensions
				.contains(extension)
			{
				connection.send(packet);
			}
		}
	}

	async fn cooldown_timer(
		connections: Weak<RwLock<Self>>,
		mut cooldown_info: CooldownInfo,
	) {
		let mut next = cooldown_info.next();
		while let Some(time) = next {
			let instant = Instant::now()
				+ time
					.duration_since(SystemTime::now())
					.unwrap_or(Duration::ZERO);
			let count = cooldown_info.pixels_available;
			tokio::time::sleep_until(instant).await;

			next = cooldown_info.next();

			let packet = packet::server::Packet::PixelsAvailable {
				count: u32::try_from(count).unwrap(),
				next: next.map(|time| {
					time.duration_since(UNIX_EPOCH)
						.unwrap()
						.as_secs()
				}),
			};

			match connections.upgrade() {
				Some(connections) => {
					let connections = connections.write().unwrap();
					connections.send(&packet);
				},
				None => {
					return;
				},
			}
		}
	}
}

#[derive(Debug, Default)]
pub struct Connections {
	by_uid: HashMap<String, Arc<RwLock<UserConnections>>>,
	by_extension: EnumMap<Extension, HashSet<Arc<AuthedSocket>>>,
}

impl Connections {
	pub async fn insert(
		&mut self,
		socket: Arc<AuthedSocket>,
		cooldown_info: Option<CooldownInfo>,
	) {
		let user = socket.user.read().await;
		if let AuthedUser::Authed { user, .. } = &*user {
			if let Some(ref id) = user.id {
				self.by_uid
					.entry(id.clone())
					.and_modify(|connections| {
						connections
							.write()
							.unwrap()
							.insert(Arc::clone(&socket))
					})
					.or_insert_with(|| {
						UserConnections::new(Arc::clone(&socket), cooldown_info.unwrap())
					});
			}
		}

		for extension in socket.extensions {
			self.by_extension[extension].insert(Arc::clone(&socket));
		}
	}

	pub async fn remove(
		&mut self,
		socket: Arc<AuthedSocket>,
	) {
		let user = socket.user.read().await;
		if let AuthedUser::Authed { user, .. } = &*user {
			if let Some(ref id) = user.id {
				let connections = self.by_uid.get(id).unwrap();
				let mut connections = connections.write().unwrap();

				connections.remove(Arc::clone(&socket));
				if connections.is_empty() {
					connections.cleanup();
					drop(connections);
					self.by_uid.remove(id);
				}
			}
		}

		for extension in socket.extensions {
			self.by_extension[extension].remove(&socket);
		}
	}

	pub fn send(
		&self,
		packet: packet::server::Packet,
	) {
		let extension = Extension::from(&packet);
		for connection in self.by_extension[extension].iter() {
			connection.send(&packet);
		}
	}

	pub fn send_to_user(
		&self,
		user_id: String,
		packet: packet::server::Packet,
	) {
		if let Some(connections) = self.by_uid.get(&user_id) {
			connections
				.read()
				.unwrap()
				.send(&packet);
		}
	}

	pub fn set_user_cooldown(
		&self,
		user_id: String,
		cooldown_info: CooldownInfo,
	) {
		if let Some(connections) = self.by_uid.get(&user_id) {
			UserConnections::set_cooldown_info(Arc::clone(connections), cooldown_info);
		}
	}

	pub fn close(&mut self) {
		// TODO: maybe send a close reason

		for connections in self.by_extension.values() {
			for connection in connections {
				connection.close();
			}
		}
	}
}

pub struct Board {
	pub id: i32,
	pub info: BoardInfo,
	connections: Connections,
	sectors: SectorCache,
}

#[derive(Clone, Debug)]
pub struct CooldownInfo {
	cooldowns: Vec<SystemTime>,
	pub pixels_available: usize,
}

impl CooldownInfo {
	fn new(
		cooldowns: Vec<SystemTime>,
		current_timestamp: SystemTime,
	) -> Self {
		let pixels_available = cooldowns
			.iter()
			.enumerate()
			.take_while(|(_, cooldown)| **cooldown <= current_timestamp)
			.last()
			.map(|(i, _)| i + 1)
			.unwrap_or(0);

		Self {
			cooldowns,
			pixels_available,
		}
	}

	pub fn into_headers(self) -> Vec<(HeaderName, HeaderValue)> {
		let mut headers = vec![(
			HeaderName::from_static("pxls-pixels-available"),
			self.pixels_available.into(),
		)];

		if let Some(next_available) = self
			.cooldowns
			.get(self.pixels_available)
		{
			headers.push((
				HeaderName::from_static("pxls-next-available"),
				(*next_available)
					.duration_since(UNIX_EPOCH)
					.unwrap()
					.as_secs()
					.into(),
			));
		}

		headers
	}

	pub fn cooldown(&self) -> Option<SystemTime> {
		self.cooldowns
			.get(self.pixels_available)
			.map(SystemTime::clone)
	}
}

impl Iterator for CooldownInfo {
	type Item = SystemTime;

	fn next(&mut self) -> Option<Self::Item> {
		let time = self.cooldown();
		if time.is_some() {
			self.pixels_available += 1;
		}
		time
	}
}

#[derive(FromPrimitive)]
pub enum MaskValue {
	NoPlace = 0,
	Place = 1,
	Adjacent = 2,
}

#[derive(Debug)]
pub enum PlaceError {
	UnknownMaskValue,
	Unplacable,
	InvalidColor,
	NoOp,
	Cooldown,
	OutOfBounds,
}

impl Reject for PlaceError {}

impl Reply for PlaceError {
	fn into_response(self) -> Response {
		match self {
			Self::UnknownMaskValue => StatusCode::INTERNAL_SERVER_ERROR,
			Self::Unplacable => StatusCode::FORBIDDEN,
			Self::InvalidColor => StatusCode::UNPROCESSABLE_ENTITY,
			Self::NoOp => StatusCode::CONFLICT,
			Self::Cooldown => StatusCode::TOO_MANY_REQUESTS,
			Self::OutOfBounds => StatusCode::NOT_FOUND,
		}
		.into_response()
	}
}

impl Board {
	pub async fn create<Connection: ConnectionTrait + TransactionTrait>(
		info: BoardInfoPost,
		connection: &Connection,
	) -> DbResult<Self> {
		let now = SystemTime::now()
			.duration_since(UNIX_EPOCH)
			.unwrap()
			.as_secs();

		let new_board = board::Entity::insert(board::ActiveModel {
				id: NotSet,
				name: Set(info.name),
				created_at: Set(now as i64),
				shape: Set(serde_json::to_value(info.shape).unwrap()),
				max_stacked: Set(info.max_pixels_available as i32),
			})
			.exec_with_returning(connection).await?;

		crate::objects::color::replace_palette(&info.palette, new_board.id, connection).await?;

		Self::load(new_board, connection).await
	}

	pub async fn read<'l, Connection: ConnectionTrait + TransactionTrait>(
		&'l self,
		buffer: SectorBuffer,
		connection: &'l Connection,
	) -> SectorCacheAccess<'l, Connection> {
		self.sectors.access(buffer, connection)
	}

	// TODO: proper error type
	pub async fn try_patch_initial<Connection: ConnectionTrait + TransactionTrait>(
		&self,
		patch: &BinaryPatch,
		connection: &Connection,
	) -> Result<(), &'static str> {
		// TODO: check bounds
		let mut sector_data = self
			.sectors
			.access(SectorBuffer::Initial, connection);

		sector_data
			.seek(SeekFrom::Start(u64::try_from(patch.start).unwrap()))
			.map_err(|_| "invalid start position")?;

		sector_data
			.write(&patch.data).await
			.map_err(|_| "write error")?;

		let packet = packet::server::Packet::BoardUpdate {
			info: None,
			data: Some(packet::server::BoardData {
				colors: None,
				timestamps: None,
				initial: Some(vec![packet::server::Change {
					position: u64::try_from(patch.start).unwrap(),
					values: Vec::from(&*patch.data),
				}]),
				mask: None,
			}),
		};

		self.connections.send(packet);

		Ok(())
	}

	pub async fn try_patch_mask<Connection: ConnectionTrait + TransactionTrait>(
		&self,
		patch: &BinaryPatch,
		connection: &Connection,
	) -> Result<(), &'static str> {
		let mut sector_data = self
			.sectors
			.access(SectorBuffer::Mask, connection);

		sector_data
			.seek(SeekFrom::Start(u64::try_from(patch.start).unwrap()))
			.map_err(|_| "invalid start position")?;

		sector_data
			.write(&patch.data).await
			.map_err(|_| "write error")?;

		let packet = packet::server::Packet::BoardUpdate {
			info: None,
			data: Some(packet::server::BoardData {
				colors: None,
				timestamps: None,
				initial: None,
				mask: Some(vec![packet::server::Change {
					position: u64::try_from(patch.start).unwrap(),
					values: Vec::from(&*patch.data),
				}]),
			}),
		};

		self.connections.send(packet);

		Ok(())
	}

	// TODO: find some way to exhaustively match info so that the compiler knows
	// when new fields are added and can notify that this function needs updates.
	pub async fn update_info<Connection: ConnectionTrait + TransactionTrait>(
		&mut self,
		info: BoardInfoPatch,
		connection: &Connection,
	) -> DbResult<()> {
		assert!(
			info.name.is_some()
				|| info.palette.is_some()
				|| info.shape.is_some()
				|| info.max_pixels_available.is_some()
		);

		let transaction = connection.begin().await?;
		if let Some(ref name) = info.name {
			board::Entity::update_many()
				.col_expr(board::Column::Name, name.into())
				.filter(board::Column::Id.eq(self.id))
				.exec(&transaction).await?;
		}

		if let Some(ref palette) = info.palette {
			replace_palette(palette, self.id, &transaction).await?;
		}

		if let Some(ref shape) = info.shape {
			board::Entity::update_many()
				.col_expr(board::Column::Shape, serde_json::to_value(shape).unwrap().into())
				.filter(board::Column::Id.eq(self.id))
				.exec(&transaction).await?;

			// TODO: try and preserve data.
			board_sector::Entity::delete_many()
				.filter(board_sector::Column::Board.eq(self.id))
				.exec(&transaction).await?;
		}

		if let Some(max_stacked) = info.max_pixels_available {
			board::Entity::update_many()
				.col_expr(board::Column::MaxStacked, (max_stacked as i32).into())
				.filter(board::Column::Id.eq(self.id))
				.exec(&transaction).await?;
		}
		
		transaction.commit().await?;

		if let Some(ref name) = info.name {
			self.info.name = name.clone();
		}

		if let Some(ref palette) = info.palette {
			self.info.palette = palette.clone();
		}

		if let Some(ref shape) = info.shape {
			self.info.shape = shape.clone();

			self.sectors = SectorCache::new(
				self.id,
				self.info.shape.sector_count(),
				self.info.shape.sector_size(),
			)
		}

		if let Some(max_stacked) = info.max_pixels_available {
			self.info.max_pixels_available = max_stacked;
		}

		let packet = packet::server::Packet::BoardUpdate {
			info: Some(info.into()),
			data: None,
		};

		self.connections.send(packet);

		Ok(())
	}

	pub async fn delete<Connection: ConnectionTrait + TransactionTrait>(
		mut self,
		connection: &Connection,
	) -> DbResult<()> {
		self.connections.close();

		let transaction = connection.begin().await?;

		board_sector::Entity::delete_many()
			.filter(board_sector::Column::Board.eq(self.id))
			.exec(&transaction).await?;

		placement::Entity::delete_many()
			.filter(placement::Column::Board.eq(self.id))
			.exec(&transaction).await?;

		color::Entity::delete_many()
			.filter(color::Column::Board.eq(self.id))
			.exec(&transaction).await?;

		board::Entity::delete_many()
			// deliberate bug to test things
			.filter(color::Column::Board.eq(self.id))
			.exec(&transaction).await?;
		
		transaction.commit().await
	}

	pub async fn last_place_time<Connection: ConnectionTrait>(
		&self,
		user: &User,
		connection: &Connection,
	) -> DbResult<u32> {
		Ok(placement::Entity::find()
			.filter(
				placement::Column::Board.eq(self.id)
					.and(placement::Column::UserId.eq(user.id.clone())),
			)
			.order_by(placement::Column::Timestamp, Order::Desc)
			.order_by(placement::Column::Id, Order::Desc)
			.one(connection).await?
			.map(|placement| placement.timestamp as u32)
			.unwrap_or(0))
	}

	pub async fn try_place<Connection: ConnectionTrait>(
		&self,
		user: &User,
		position: u64,
		color: u8,
		connection: &Connection,
	) -> Result<placement::Model, DatabaseError<PlaceError>> {
		// TODO: I hate most things about how this is written. Redo it and/or move
		// stuff.

		let (sector_index, sector_offset) = self
			.info
			.shape
			.to_local(position as usize)
			.ok_or(PlaceError::OutOfBounds)?;

		if !self.info.palette.contains_key(&(color as u32)) {
			return Err(PlaceError::InvalidColor.into());
		}
		
		let mut sector = self
			.sectors
			.write_sector(sector_index, connection).await
			.expect("Failed to load sector");

		match FromPrimitive::from_u8(sector.mask[sector_offset]) {
			Some(MaskValue::Place) => Ok(()),
			Some(MaskValue::NoPlace) => Err(PlaceError::Unplacable),
			// NOTE: there exists an old implementation in the version
			// control history. It's messy and would need to load adjacent
			// sectors now so I'm dropping it for now.
			Some(MaskValue::Adjacent) => unimplemented!(),
			None => Err(PlaceError::UnknownMaskValue),
		}?;

		if sector.colors[sector_offset] == color {
			return Err(PlaceError::NoOp.into());
		}

		let timestamp = self.current_timestamp();
		let cooldown_info = self.user_cooldown_info(user, connection).await
			.map_err(DatabaseError::DbErr)?;

		if cooldown_info.pixels_available == 0 {
			return Err(PlaceError::Cooldown.into());
		}

		let new_placement = placement::Entity::insert(
				placement::ActiveModel {
					id: NotSet,
					board: Set(self.id),
					position: Set(position as i64),
					color: Set(color as i16),
					timestamp: Set(timestamp as i32),
					user_id: Set(user.id.clone()),
				}
			)
			.exec_with_returning(connection).await
			.expect("failed to insert placement");

		sector.colors[sector_offset] = color;
		let timestamp_slice =
			&mut sector.timestamps[(sector_offset * 4)..((sector_offset + 1) * 4)];
		timestamp_slice
			.as_mut()
			.put_u32_le(timestamp);

		let packet = packet::server::Packet::BoardUpdate {
			info: None,
			data: Some(packet::server::BoardData {
				colors: Some(vec![packet::server::Change {
					position,
					values: vec![color],
				}]),
				timestamps: Some(vec![packet::server::Change {
					position,
					values: vec![timestamp],
				}]),
				initial: None,
				mask: None,
			}),
		};

		self.connections.send(packet);

		if let Some(user_id) = user.id.clone() {
			let cooldown_info = self
				.user_cooldown_info(user, connection).await
				.map_err(DatabaseError::DbErr)?;

			self.connections
				.set_user_cooldown(user_id, cooldown_info);
		}

		Ok(new_placement)
	}

	pub async fn list_placements<Connection: ConnectionTrait>(
		&self,
		timestamp: u32,
		id: usize,
		limit: usize,
		reverse: bool,
		connection: &Connection,
	) -> DbResult<Vec<placement::Model>> {
		let column_timestamp_id_pair = Expr::tuple([
			Expr::col(placement::Column::Timestamp).into(),
			Expr::col(placement::Column::Id).into(),
		]);

		let value_timestamp_id_pair = Expr::tuple([
			(timestamp as i32).into(),
			(id as i32).into(),
		]);

		let compare = if reverse {
			Expr::lt(column_timestamp_id_pair.clone(), value_timestamp_id_pair)
		} else {
			Expr::gte(column_timestamp_id_pair.clone(), value_timestamp_id_pair)
		};

		let order = if reverse { Order::Desc } else { Order::Asc };

		placement::Entity::find()
			.filter(placement::Column::Board.eq(self.id))
			.filter(compare)
			.order_by(column_timestamp_id_pair, order)
			.limit(limit as u64)
			.all(connection).await
	}

	pub async fn lookup<Connection: ConnectionTrait>(
		&self,
		position: u64,
		connection: &Connection,
	) -> DbResult<Option<placement::Model>> {
		placement::Entity::find()
			.filter(
				placement::Column::Board.eq(self.id)
					.and(placement::Column::Position.eq(position as i64)),
			)
			.order_by(placement::Column::Timestamp, Order::Desc)
			.order_by(placement::Column::Id, Order::Desc)
			.one(connection).await
	}

	pub async fn load<Connection: ConnectionTrait>(
		board: board::Model,
		connection: &Connection,
	) -> DbResult<Self> {
		let id = board.id;

		let palette: Palette = board.find_related(color::Entity)
			.all(connection).await?
			.into_iter()
			.map(|color| (color.index as u32, Color::from(color)))
			.collect();

		let info = BoardInfo {
			name: board.name.clone(),
			created_at: board.created_at as u64,
			shape: serde_json::from_value(board.shape).unwrap(),
			palette,
			max_pixels_available: board.max_stacked as u32,
		};

		let sectors = SectorCache::new(
			board.id,
			info.shape.sector_count(),
			info.shape.sector_size(),
		);

		let connections = Connections::default();

		Ok(Board {
			id,
			info,
			sectors,
			connections,
		})
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

	async fn pixel_density_at_time<Connection: ConnectionTrait>(
		&self,
		position: u64,
		timestamp: u32,
		connection: &Connection,
	) -> DbResult<usize> {
		placement::Entity::find()
			.filter(
				placement::Column::Position.eq(position as i64)
				.and(placement::Column::Timestamp.lt(timestamp as i32)),
			)
			.count(connection).await
			.map(|i| i as usize)
	}

	// TODO: This should REALLY be cached.
	// It's very heavy for how often it should be used, but values should
	// continue to be valid until the cooldown formula itself changes.
	async fn calculate_cooldowns<Connection: ConnectionTrait>(
		&self,
		placement: Option<&placement::Model>,
		connection: &Connection,
	) -> DbResult<Vec<SystemTime>> {
		// this is pretty ugly
		// TODO: generalize for more cooldown variables
		let (activity, density) = if let Some(placement) = placement {
			(
				self.user_count_for_time(placement.timestamp as u32, connection).await?.active,
				self.pixel_density_at_time(
					placement.position as u64,
					placement.timestamp as u32,
					connection,
				).await?,
			)
		} else {
			(0, 0)
		};

		let board_time = self.info.created_at;

		// TODO: proper cooldown
		Ok(std::iter::repeat(30)
			.enumerate()
			.map(|(i, c)| u32::try_from((i + 1) * c).unwrap())
			.zip(std::iter::repeat(
				placement
					.map(|p| p.timestamp as u32)
					.unwrap_or(0),
			))
			.map(|(a, b)| a + b)
			.take(usize::try_from(self.info.max_pixels_available).unwrap())
			.map(|offset| board_time + offset as u64)
			.map(Duration::from_secs)
			.map(|offset| UNIX_EPOCH + offset)
			.collect())
	}

	async fn recent_user_placements<Connection: ConnectionTrait>(
		&self,
		user: &User,
		limit: usize,
		connection: &Connection,
	) -> DbResult<Vec<placement::Model>> {
		Ok(placement::Entity::find()
			.filter(
				placement::Column::Board.eq(self.id)
					.and(placement::Column::UserId.eq(user.id.clone())),
			)
			.order_by(placement::Column::Timestamp, Order::Desc)
			.order_by(placement::Column::Id, Order::Desc)
			.limit(Some(limit as u64))
			.all(connection).await?
			.into_iter()
			.rev()
			.collect::<Vec<_>>())
	}

	// TODO: If any code here is a mess, this certainly is.
	// The explanations don't even make sense: just make it readable.
	pub async fn user_cooldown_info<Connection: ConnectionTrait>(
		&self,
		user: &User,
		connection: &Connection,
	) -> DbResult<CooldownInfo> {
		let placements = self.recent_user_placements(
			user,
			usize::try_from(self.info.max_pixels_available).unwrap(),
			connection,
		).await?;

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
							u64::from(pair[1].timestamp as u32) + self.info.created_at,
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

	async fn user_count_for_time<Connection: ConnectionTrait>(
		&self,
		timestamp: u32,
		connection: &Connection,
	) -> DbResult<UserCount> {
		// TODO: make configurable
		let idle_timeout = 5 * 60;
		let max_time = i32::try_from(timestamp).unwrap();
		let min_time = i32::try_from(timestamp.saturating_sub(idle_timeout)).unwrap();

		let count = placement::Entity::find()
			.distinct_on([placement::Column::UserId])
			.filter(placement::Column::Board.eq(self.id))
			.filter(placement::Column::Timestamp.between(min_time, max_time))
			.count(connection).await?;

		Ok(UserCount {
			idle_timeout,
			active: count as usize,
		})
	}

	pub async fn user_count<Connection: ConnectionTrait>(
		&self,
		connection: &Connection,
	) -> DbResult<UserCount> {
		self.user_count_for_time(self.current_timestamp(), connection).await
	}

	pub async fn insert_socket<Connection: ConnectionTrait>(
		&mut self,
		socket: Arc<AuthedSocket>,
		connection: &Connection,
	) -> DbResult<()> {
		let user = socket.user.read().await;
		let user = Option::<&User>::from(&*user);

		let cooldown_info = if let Some(user) = user {
			if user.id.is_some() {
				Some(self.user_cooldown_info(user, connection).await?)
			} else {
				None
			}
		} else {
			None
		};

		self.connections.insert(Arc::clone(&socket), cooldown_info).await;
		socket.send(&packet::server::Packet::Ready).await;

		Ok(())
	}

	pub async fn remove_socket(
		&mut self,
		socket: Arc<AuthedSocket>,
	) {
		self.connections.remove(socket).await
	}
}

impl From<&Board> for Uri {
	fn from(board: &Board) -> Self {
		format!("/boards/{}", board.id)
			.parse::<Uri>()
			.unwrap()
	}
}

impl<'l> From<&'l Board> for Reference<'l, BoardInfo> {
	fn from(board: &'l Board) -> Self {
		Self {
			uri: board.into(),
			view: &board.info,
		}
	}
}
