use std::{
	convert::TryFrom,
	io::{Seek, SeekFrom},
	sync::Arc,
	time::{Duration, SystemTime, UNIX_EPOCH},
};

use bytes::BufMut;
use http::{StatusCode, Uri};
use num_derive::FromPrimitive;
use num_traits::FromPrimitive;
use sea_orm::{
	ConnectionTrait,
	Set,
	ActiveValue::NotSet,
	EntityTrait,
	ColumnTrait,
	QueryFilter,
	QueryOrder,
	QuerySelect,
	sea_query::Expr,
	ModelTrait,
	PaginatorTrait,
	TransactionTrait,
};
use serde::{Deserialize, Serialize};
use warp::{reject::Reject, reply::Response, Reply};

use crate::{
	filters::body::patch::BinaryPatch,
	database::{DbResult, entities::*},
	DatabaseError,
};

use super::packet;
use super::connections::Connections;
use super::color::*;
use super::sector_cache::*;
use super::{
	VecShape,
	SectorBuffer,
	Shape,
	User,
	AuthedSocket,
	UserCount,
	Reference,
	CooldownInfo,
};

#[derive(Debug, Clone, Copy)]
pub enum Order {
	Forward,
	Reverse,
}

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
	fn from(info: BoardInfoPatch) -> Self {
		Self {
			name: info.name,
			shape: info.shape,
			palette: info.palette,
			max_pixels_available: info.max_pixels_available,
		}
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

pub struct Board {
	pub id: i32,
	pub info: BoardInfo,
	connections: Connections,
	sectors: SectorCache,
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

		let initial = packet::server::Change {
			position: u64::try_from(patch.start).unwrap(),
			values: Vec::from(&*patch.data),
		};
		
		let data = packet::server::BoardData::builder()
			.initial(vec![initial]);

		self.connections.send_boarddata(data).await;

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

		let mask = packet::server::Change {
			position: u64::try_from(patch.start).unwrap(),
			values: Vec::from(&*patch.data),
		};

		let data = packet::server::BoardData::builder()
			.mask(vec![mask]);

		self.connections.send_boarddata(data).await;

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

		self.connections.send(packet).await;

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
			.order_by(placement::Column::Timestamp, sea_orm::Order::Desc)
			.order_by(placement::Column::Id, sea_orm::Order::Desc)
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

		let color = packet::server::Change {
			position,
			values: vec![color],
		};

		let timestamp = packet::server::Change {
			position,
			values: vec![timestamp],
		};

		let data = packet::server::BoardData::builder()
			.colors(vec![color])
			.timestamps(vec![timestamp]);

		self.connections.send_boarddata(data).await;

		if let Some(user_id) = user.id.clone() {
			let cooldown_info = self
				.user_cooldown_info(user, connection).await
				.map_err(DatabaseError::DbErr)?;

			self.connections.set_user_cooldown(user_id, cooldown_info).await;
		}

		Ok(new_placement)
	}

	pub async fn list_placements<Connection: ConnectionTrait>(
		&self,
		timestamp: u32,
		id: usize,
		limit: usize,
		order: Order,
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

		let compare_lhs = column_timestamp_id_pair.clone();
		let compare_rhs = value_timestamp_id_pair;
		let compare = match order {
			Order::Forward => Expr::lt(compare_lhs, compare_rhs),
			Order::Reverse => Expr::gte(compare_lhs, compare_rhs),
		};

		let order = match order {
			Order::Forward => sea_orm::Order::Asc,
			Order::Reverse => sea_orm::Order::Desc,
		};

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
			.order_by(placement::Column::Timestamp, sea_orm::Order::Desc)
			.order_by(placement::Column::Id, sea_orm::Order::Desc)
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
		let parameters = if let Some(placement) = placement {
			let activity = self.user_count_for_time(
				placement.timestamp as u32,
				connection
			).await?.active;

			let density = self.pixel_density_at_time(
				placement.position as u64,
				placement.timestamp as u32,
				connection,
			).await?;

			let timestamp = placement.timestamp as u32;
			
			(activity, density, timestamp)
		} else {
			(0, 0, 0)
		};

		let (activity, density, timestamp) = parameters;

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
			.order_by(placement::Column::Timestamp, sea_orm::Order::Desc)
			.order_by(placement::Column::Id, sea_orm::Order::Desc)
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
		socket: &Arc<AuthedSocket>,
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

		self.connections.insert(socket, cooldown_info).await;
		socket.send(&packet::server::Packet::Ready).await;

		Ok(())
	}

	pub async fn remove_socket(
		&mut self,
		socket: &Arc<AuthedSocket>,
	) {
		self.connections.remove(socket).await
	}
}