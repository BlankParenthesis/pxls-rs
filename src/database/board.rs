use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use sea_orm::FromQueryResult;
use warp::http::Uri;

use sea_orm::{ActiveValue::NotSet, ColumnTrait, ConnectionTrait, EntityTrait, ModelTrait, QueryFilter, QueryOrder, QuerySelect, Set, StreamTrait, TransactionTrait};
use sea_query::SimpleExpr;
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, RwLock};

use crate::config::CONFIG;
use crate::board::{CooldownCache, Shape};
use crate::filter::response::reference::Referencable;
use crate::routes::placement_statistics::users::PlacementColorStatistics;
use crate::board::{ActivityCache, Board};

use super::entities::*;

use super::{Connection, DbResult, Database};
use super::specifier::{PathPart, Specifier, SpecifierParser, SpecfierParseError, Id, specifier_path};
use super::user::UserSpecifier;
use super::color::{Palette, Color};
use super::placement::CachedPlacement;

pub struct BoardListSpecifier;

impl Specifier for BoardListSpecifier {
	fn filter(&self) -> SimpleExpr {
		unimplemented!()
	}
	
	fn from_ids(_: &[&str]) -> Result<Self, SpecfierParseError> {
		Ok(Self)
	}
	
	fn to_ids(&self) -> Box<[Id]> {
		Box::new([])
	}

	fn parts() -> &'static [PathPart] {
		specifier_path!("boards")
	}
}

impl<'de> Deserialize<'de> for BoardListSpecifier {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where D: serde::Deserializer<'de> {
		deserializer.deserialize_str(SpecifierParser::new("A board list uri"))
	}
}

impl Serialize for BoardListSpecifier {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where S: serde::Serializer {
		serializer.serialize_str(self.to_uri().path())
	}
}

#[derive(Debug, Eq, PartialEq, Clone, Copy, Hash)]
pub struct BoardSpecifier(pub i32);

impl Specifier for BoardSpecifier {
	fn filter(&self) -> SimpleExpr {
		board::Column::Id.eq(self.0)
	}
	
	fn from_ids(ids: &[&str]) -> Result<Self, SpecfierParseError> {
		let board = ids[0].parse()?;
		Ok(Self(board))
	}
	
	fn to_ids(&self) -> Box<[Id]> {
		Box::new([Id::I32(self.0)])
	}

	fn parts() -> &'static [PathPart] {
		specifier_path!("boards", board)
	}
}

impl<'de> Deserialize<'de> for BoardSpecifier {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where D: serde::Deserializer<'de> {
		deserializer.deserialize_str(SpecifierParser::new("A board uri"))
	}
}

impl Serialize for BoardSpecifier {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where S: serde::Serializer {
		serializer.serialize_str(self.to_uri().path())
	}
}

#[derive(Serialize, Debug, Clone)]
pub struct BoardInfo {
	#[serde(skip_serializing)]
	pub id: BoardSpecifier,
	pub name: String,
	pub created_at: u64,
	pub shape: Shape,
	pub palette: Palette,
	pub max_pixels_available: u32,
}

impl BoardInfo {
	fn from_model(model: board::Model, palette: Palette) -> Self {
		BoardInfo {
			id: BoardSpecifier(model.id),
			name: model.name,
			created_at: model.created_at as _,
			shape: serde_json::from_value(model.shape).unwrap(),
			palette,
			max_pixels_available: model.max_stacked as _,
		}
	}
}

impl Referencable for BoardInfo {
	fn uri(&self) -> Uri {
		self.id.to_uri()
	}
}

#[derive(Debug, FromQueryResult)]
pub struct UserStats {
	user_id: i32,
	color: i16,
	count: i64,
}

impl<C: TransactionTrait + ConnectionTrait + StreamTrait> Connection<C> {
	pub async fn list_boards(&self) -> DbResult<Vec<BoardInfo>> {
		let transaction = self.begin().await?;
		
		let infos = board::Entity::find()
			.all(&transaction.connection).await?;
		
		let mut boards = Vec::with_capacity(infos.len());
		for info in infos {
			let palette = info.find_related(color::Entity)
				.all(&transaction.connection).await?
				.into_iter()
				.map(|color| (color.index as u32, Color::from(color)))
				.collect();
			
			boards.push(BoardInfo::from_model(info, palette));
		}
		
		transaction.commit().await?;

		Ok(boards)
	}
	
	async fn all_stats(
		&self,
		board: &BoardSpecifier,
	) -> DbResult<HashMap<UserSpecifier, PlacementColorStatistics>> {
		
		let stats = placement::Entity::find()
			.select_only()
			.column(placement::Column::UserId)
			.column(placement::Column::Color)
			.column_as(placement::Column::Timestamp.count(), "count")
			.group_by(placement::Column::UserId)
			.group_by(placement::Column::Color)
			.filter(placement::Column::Board.eq(board.0))
			.into_model::<UserStats>()
			.all(&self.connection).await?;

		let mut stats_by_user = HashMap::<_, PlacementColorStatistics>::new();

		for stat in stats {
			let user = UserSpecifier(stat.user_id);
			let user_stats = stats_by_user.entry(user).or_default();

			let color_stats = user_stats.colors.entry(stat.color as _).or_default();
			color_stats.placed += stat.count as usize;
		}

		Ok(stats_by_user)
	}
	
	async fn load_cache(
		&self,
		info: &BoardInfo,
	) -> DbResult<(ActivityCache, CooldownCache)> {
		// TODO: make configurable
		const IDLE_TIMEOUT: u32 = 5 * 60;
		
		let unix_time = SystemTime::now()
			.duration_since(SystemTime::UNIX_EPOCH).unwrap()
			.as_secs();
		let timestamp: u32 = unix_time.saturating_sub(info.created_at).max(1)
 			.try_into().unwrap();
		let epoch = SystemTime::now() - Duration::from_secs(timestamp as u64);
		
		// the point after which activity from users will be considered currently active
		let idle_begin = timestamp.saturating_sub(IDLE_TIMEOUT);
		
		let max_stack_cooldown = info.max_pixels_available * CONFIG.cooldown;
		// the point after which users may possibly not have a full stack of pixels
		let cooldown_begin = timestamp - max_stack_cooldown;
		
		let mut activity_cache = ActivityCache::new(IDLE_TIMEOUT);
		let mut cooldown_cache = CooldownCache::new(info.max_pixels_available, epoch);
		
		let placements = placement::Entity::find()
			.filter(placement::Column::Board.eq(info.id.0))
			.filter(placement::Column::Timestamp.gt(cooldown_begin)
				.or(placement::Column::Timestamp.gt(idle_begin)))
			.order_by(placement::Column::Timestamp, sea_orm::Order::Desc)
			.order_by(placement::Column::Id, sea_orm::Order::Desc)
			.all(&self.connection).await?
			.into_iter()
			.map(CachedPlacement::from)
			.rev();
		
		for placement in placements {
			// TODO
			let density = 0;
			let timestamp = placement.modified;
			
			activity_cache.insert(timestamp, placement.user);
			let activity = activity_cache.count(timestamp) as u32;
			
			cooldown_cache.insert(timestamp, placement.user, activity, density);
		}
		
		Ok((activity_cache, cooldown_cache))
	}
	
	pub async fn board_from_model(
		&self,
		info: BoardInfo,
		pool: Arc<Database>,
	) -> DbResult<Board> {
		let transaction = self.begin().await?;

		let statistics_cache = transaction.all_stats(&info.id).await?.into();
		
		let (activity, cooldown) = transaction.load_cache(&info).await?;
		
		transaction.commit().await?;
		
		let activity_cache = Mutex::new(activity);
		let cooldown_cache = RwLock::new(cooldown);
		
		Ok(Board::new(
			info,
			statistics_cache,
			activity_cache,
			cooldown_cache,
			pool,
		))
	}
	
	pub async fn create_board(
		&self,
		name: String,
		shape: Vec<Vec<usize>>,
		palette: Palette,
		max_pixels_available: u32,
		pool: Arc<Database>,
	) -> DbResult<Board> {
		let now = SystemTime::now()
			.duration_since(UNIX_EPOCH)
			.unwrap()
			.as_secs();

		let transaction = self.begin().await?;
		
		let model = board::ActiveModel {
			id: NotSet,
			name: Set(name),
			created_at: Set(now as i64),
			shape: Set(serde_json::to_value(shape).unwrap()),
			max_stacked: Set(max_pixels_available as i32),
		};

		let new_board = board::Entity::insert(model)
			.exec_with_returning(&transaction.connection).await?;

		transaction.replace_palette(palette.clone(), new_board.id).await?;
		transaction.commit().await?;
		
		let info = BoardInfo::from_model(new_board, palette);
		let board = self.board_from_model(info, pool).await?;

		Ok(board)
	}
	
	pub async fn edit_board(
		&self,
		board: BoardSpecifier,
		name: Option<String>,
		shape: Option<Vec<Vec<usize>>>,
		palette: Option<Palette>,
		max_pixels_available: Option<u32>,
	) -> DbResult<()> {
		let transaction = self.begin().await?;
		
		if let Some(ref name) = name {
			board::Entity::update_many()
				.col_expr(board::Column::Name, name.into())
				.filter(board.filter())
				.exec(&transaction.connection).await?;
		}

		if let Some(palette) = palette {
			transaction.replace_palette(palette, board.0).await?;
		}

		if let Some(ref shape) = shape {
			board::Entity::update_many()
				.col_expr(board::Column::Shape, serde_json::to_value(shape).unwrap().into())
				.filter(board.filter())
				.exec(&transaction.connection).await?;

			// TODO: try and preserve data.
			board_sector::Entity::delete_many()
				.filter(board_sector::Column::Board.eq(board.0))
				.exec(&transaction.connection).await?;
		}

		if let Some(max_stacked) = max_pixels_available {
			board::Entity::update_many()
				.col_expr(board::Column::MaxStacked, (max_stacked as i32).into())
				.filter(board.filter())
				.exec(&transaction.connection).await?;
		}
		
		transaction.commit().await?;

		Ok(())
	}
	
	pub async fn delete_board(&self, board: BoardSpecifier) -> DbResult<()> {
		let transaction = self.begin().await?;

		board_sector::Entity::delete_many()
			.filter(board_sector::Column::Board.eq(board.0))
			.exec(&transaction.connection).await?;

		placement::Entity::delete_many()
			.filter(placement::Column::Board.eq(board.0))
			.exec(&transaction.connection).await?;

		color::Entity::delete_many()
			.filter(color::Column::Board.eq(board.0))
			.exec(&transaction.connection).await?;

		board::Entity::delete_many()
			.filter(board.filter())
			.exec(&transaction.connection).await?;
		
		transaction.commit().await?;

		Ok(())
	}
	
}
