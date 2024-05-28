use std::{time::{Duration, SystemTime, UNIX_EPOCH}, collections::HashMap};

use bytes::{BytesMut, BufMut};
use reqwest::StatusCode;
use sea_orm::{
	ConnectOptions, 
	Database, 
	DatabaseConnection, 
	DbErr, 
	TransactionTrait, 
	DatabaseTransaction,
	EntityTrait,
	ColumnTrait,
	QueryFilter,
	Set,
	ModelTrait,
	ActiveValue::NotSet,
	sea_query::{Expr, SimpleExpr, self},
	QuerySelect,
	QueryOrder,
	PaginatorTrait,
	Iden,
	ConnectionTrait, QueryTrait,
};
use sea_orm_migration::MigratorTrait;
use tokio::sync::RwLock;
use warp::reply::Reply;

use crate::{config::CONFIG, filter::response::paginated_list::Page, routes::{site_notices::notices::{Notice, NoticeFilter}, board_notices::boards::notices::{BoardsNoticePageToken, BoardsNotice, BoardNoticeFilter}, core::boards::pixels::PlacementFilter}, board::CachedPlacement};
use crate::board::{Palette, Color, Board, Placement, PlacementPageToken, Sector};
use crate::routes::site_notices::notices::NoticePageToken;

mod entities;

use entities::*;
use migration::Migrator;

use super::Order;

#[derive(Debug)]
pub enum BoardsDatabaseError {
	DbErr(sea_orm::DbErr),
}

impl From<sea_orm::DbErr> for BoardsDatabaseError {
	fn from(value: sea_orm::DbErr) -> Self {
		BoardsDatabaseError::DbErr(value)
	}
}

impl From<&BoardsDatabaseError> for StatusCode {
	fn from(error: &BoardsDatabaseError) -> Self {
		match error {
			BoardsDatabaseError::DbErr(err) => {
				StatusCode::INTERNAL_SERVER_ERROR
			}
		}
	}
}

impl From<BoardsDatabaseError> for StatusCode {
	fn from(error: BoardsDatabaseError) -> Self {
		error.into()
	}
}

impl Reply for BoardsDatabaseError {
	fn into_response(self) -> warp::reply::Response {
		StatusCode::from(&self).into_response()
	}
}

type DbResult<T> = Result<T, BoardsDatabaseError>;

pub struct BoardsDatabase {
	pool: DatabaseConnection,
}

#[async_trait::async_trait]
impl super::Database for BoardsDatabase {
	type Error = DbErr;
	type Connection = BoardsConnection<DatabaseConnection>;

	async fn connect() -> Result<Self, Self::Error> {
		let url = CONFIG.database_url.to_string();
		let mut connect_options = ConnectOptions::new(url);
		connect_options
			.connect_timeout(Duration::from_secs(2))
			.acquire_timeout(Duration::from_secs(2));
		
		let pool = Database::connect(connect_options).await?;
		Migrator::up(&pool, None).await?;
		Ok(Self { pool })
	}

	async fn connection(&self) -> Result<Self::Connection, Self::Error> {
		let connection = self.pool.clone();
		Ok(BoardsConnection { connection })
	}
}

#[derive(Default)]
struct UserIdCache {
	data: RwLock<(HashMap<i32, String>, HashMap<String, i32>)>,
}

impl UserIdCache {
	async fn get_uid<C: ConnectionTrait>(
		&self,
		id: i32,
		connection: &C,
	) -> Result<String, BoardsDatabaseError> {
		let cache = self.data.read().await;
		if let Some(uid) = cache.0.get(&id) {
			Ok(uid.clone())
		} else {
			drop(cache);
			let mut cache = self.data.write().await;
			let user = user_id::Entity::find_by_id(id)
				.one(connection).await?.unwrap();
			cache.0.insert(user.id, user.uid.clone());
			cache.1.insert(user.uid.clone(), user.id);
			Ok(user.uid.clone())
		}
	}

	async fn get_id<C: ConnectionTrait>(
		&self,
		uid: String,
		connection: &C,
	) -> Result<i32, BoardsDatabaseError> {
		let cache = self.data.read().await;
		if let Some(&id) = cache.1.get(&uid) {
			Ok(id)
		} else {
			drop(cache);
			let mut cache = self.data.write().await;
			let new_user = user_id::ActiveModel {
				id: NotSet,
				uid: Set(uid.clone()),
			};
			let user = user_id::Entity::insert(new_user)
				.on_conflict(
					sea_query::OnConflict::column(user_id::Column::Uid)
						.update_column(user_id::Column::Uid)
						.to_owned()
				)
				.exec_with_returning(connection).await?;
			cache.0.insert(user.id, user.uid.clone());
			cache.1.insert(user.uid, user.id);
			Ok(user.id)
		}
	}
}

lazy_static! {
	static ref USER_ID_CACHE: UserIdCache = UserIdCache::default();
}

pub struct BoardsConnection<Connection: TransactionTrait + ConnectionTrait> {
	connection: Connection,
}

impl BoardsConnection<DatabaseTransaction> {
	pub async fn commit(self) -> DbResult<()> {
		self.connection.commit().await
			.map_err(BoardsDatabaseError::from)
	}
}

impl<C: TransactionTrait + ConnectionTrait> BoardsConnection<C> {
	pub async fn begin(&self) -> DbResult<BoardsConnection<DatabaseTransaction>> {
		self.connection.begin().await
			.map(|connection| BoardsConnection { connection })
			.map_err(BoardsDatabaseError::from)
	}

	pub async fn get_uid(&self, user_id: &str) -> Result<i32, BoardsDatabaseError> {
		USER_ID_CACHE.get_id(user_id.to_owned(), &self.connection).await
	}

	pub async fn list_boards(&self) -> DbResult<Vec<Board>> {
		
		let db_boards = board::Entity::find()
			.all(&self.connection).await?;

		let mut boards = Vec::with_capacity(db_boards.len());

		for board in db_boards {
			boards.push(self.board_from_model(board).await?);
		}

		Ok(boards)
	}

	async fn board_from_model(&self, board: board::Model) -> DbResult<Board> {
		let id = board.id;

		let palette: Palette = board.find_related(color::Entity)
			.all(&self.connection).await?
			.into_iter()
			.map(|color| {
				let index = color.index as u32;
				let color = Color {
					name: color.name,
					value: color.value as u32,
					system_only: color.system_only,
				};

				(index, color)
			})
			.collect();
		
		Ok(Board::new(
			id,
			board.name,
			board.created_at as u64,
			serde_json::from_value(board.shape).unwrap(),
			palette,
			board.max_stacked as u32,
		))
	}

	pub async fn create_board(
		&self,
		name: String,
		shape: Vec<Vec<usize>>,
		palette: Palette,
		max_pixels_available: u32,
	) -> DbResult<Board> {
		let now = SystemTime::now()
			.duration_since(UNIX_EPOCH)
			.unwrap()
			.as_secs();

		let transaction = self.begin().await?;

		let new_board = board::Entity::insert(board::ActiveModel {
				id: NotSet,
				name: Set(name),
				created_at: Set(now as i64),
				shape: Set(serde_json::to_value(shape).unwrap()),
				max_stacked: Set(max_pixels_available as i32),
			})
			.exec_with_returning(&transaction.connection).await?;

		transaction.replace_palette(palette, new_board.id).await?;

		
		let board = self.board_from_model(new_board).await?;

		transaction.commit().await?;

		Ok(board)
	}

	async fn replace_palette(
		&self,
		palette: Palette,
		board_id: i32,
	) -> DbResult<()> {
		let transaction = self.begin().await?;

		color::Entity::delete_many()
			.filter(color::Column::Board.eq(board_id))
			.exec(&transaction.connection).await?;
	
		for (index, Color { name, value, system_only }) in palette {
			let color = color::ActiveModel {
				board: Set(board_id),
				index: Set(index as i32),
				name: Set(name.clone()),
				value: Set(value as i32),
				system_only: Set(system_only),
			};
	
			color::Entity::insert(color)
				.exec(&transaction.connection).await?;
		}
		
		transaction.commit().await?;

		Ok(())
	}

	pub async fn update_board_info(
		&self,
		board_id: i32,
		name: Option<String>,
		shape: Option<Vec<Vec<usize>>>,
		palette: Option<Palette>,
		max_pixels_available: Option<u32>,
	) -> DbResult<()> {
		let transaction = self.begin().await?;
		
		if let Some(ref name) = name {
			board::Entity::update_many()
				.col_expr(board::Column::Name, name.into())
				.filter(board::Column::Id.eq(board_id))
				.exec(&transaction.connection).await?;
		}

		if let Some(palette) = palette {
			transaction.replace_palette(palette, board_id).await?;
		}

		if let Some(ref shape) = shape {
			board::Entity::update_many()
				.col_expr(board::Column::Shape, serde_json::to_value(shape).unwrap().into())
				.filter(board::Column::Id.eq(board_id))
				.exec(&transaction.connection).await?;

			// TODO: try and preserve data.
			board_sector::Entity::delete_many()
				.filter(board_sector::Column::Board.eq(board_id))
				.exec(&transaction.connection).await?;
		}

		if let Some(max_stacked) = max_pixels_available {
			board::Entity::update_many()
				.col_expr(board::Column::MaxStacked, (max_stacked as i32).into())
				.filter(board::Column::Id.eq(board_id))
				.exec(&transaction.connection).await?;
		}
		
		transaction.commit().await?;

		Ok(())
	}
	
	pub async fn delete_board(&self, board_id: i32) -> DbResult<()> {
		let transaction = self.begin().await?;

		board_sector::Entity::delete_many()
			.filter(board_sector::Column::Board.eq(board_id))
			.exec(&transaction.connection).await?;

		placement::Entity::delete_many()
			.filter(placement::Column::Board.eq(board_id))
			.exec(&transaction.connection).await?;

		color::Entity::delete_many()
			.filter(color::Column::Board.eq(board_id))
			.exec(&transaction.connection).await?;

		board::Entity::delete_many()
			.filter(board::Column::Id.eq(board_id))
			.exec(&transaction.connection).await?;
		
		transaction.commit().await?;

		Ok(())
	}

	pub async fn last_place_time(
		&self,
		board_id: i32,
		user_id: String,
	) -> DbResult<u32> {
		placement::Entity::find()
			.filter(
				placement::Column::Board.eq(board_id)
					.and(placement::Column::UserId.eq(user_id)),
			)
			.order_by(placement::Column::Timestamp, sea_orm::Order::Desc)
			.order_by(placement::Column::Id, sea_orm::Order::Desc)
			.one(&self.connection).await
			.map(|option| option.map(|placement| placement.timestamp))
			.map(|timestamp| timestamp.unwrap_or(0) as u32)
			.map_err(BoardsDatabaseError::from)
	}

	pub async fn list_placements_simple(
		&self,
		board_id: i32,
		order: Order,
		limit: usize,
	) -> DbResult<Vec<CachedPlacement>> {
		let order = match order {
			Order::Forward => sea_orm::Order::Asc,
			Order::Reverse => sea_orm::Order::Desc,
		};

		Ok(placement::Entity::find()
			.filter(placement::Column::Board.eq(board_id))
			.order_by(placement::Column::Timestamp, order.clone())
			.order_by(placement::Column::Id, order)
			.limit(limit as u64)
			.all(&self.connection).await?
			.into_iter()
			.map(|placement| CachedPlacement {
				timestamp: placement.timestamp as u32,
				position: placement.position as u64,
				user_id: placement.user_id,
			})
			.collect())
	}

	pub async fn list_placements(
		&self,
		board_id: i32,
		token: PlacementPageToken,
		limit: usize,
		order: Order,
		filter: PlacementFilter,
	) -> DbResult<Page<Placement>> {
		let column_timestamp_id_pair = Expr::tuple([
			Expr::col(placement::Column::Timestamp).into(),
			Expr::col(placement::Column::Id).into(),
		]);

		let value_timestamp_id_pair = Expr::tuple([
			(token.timestamp as i32).into(),
			(token.id as i32).into(),
		]);

		let compare_lhs = column_timestamp_id_pair.clone();
		let compare_rhs = value_timestamp_id_pair;
		let compare = match order {
			Order::Forward => Expr::gt(compare_lhs, compare_rhs),
			Order::Reverse => Expr::lt(compare_lhs, compare_rhs),
		};

		let order = match order {
			Order::Forward => sea_orm::Order::Asc,
			Order::Reverse => sea_orm::Order::Desc,
		};

		let placements = placement::Entity::find()
			.filter(placement::Column::Board.eq(board_id))
			.filter(compare)
			.apply_if(filter.color.start, |q, start| q.filter(placement::Column::Color.gte(start)))
			.apply_if(filter.color.end, |q, end| q.filter(placement::Column::Color.lte(end)))
			.apply_if(filter.user.as_ref(), |q, id| q.filter(placement::Column::UserId.eq(id)))
			.apply_if(filter.position.start, |q, start| q.filter(placement::Column::Position.gte(start)))
			.apply_if(filter.position.end, |q, end| q.filter(placement::Column::Position.lte(end)))
			.apply_if(filter.timestamp.start, |q, start| q.filter(placement::Column::Timestamp.gte(start)))
			.apply_if(filter.timestamp.end, |q, end| q.filter(placement::Column::Timestamp.lte(end)))
			.order_by(column_timestamp_id_pair, order)
			.limit(limit as u64 + 1) // fetch one extra to see if this is the end of the data
			.all(&self.connection).await?;


		let next = placements.windows(2).nth(limit.saturating_sub(1))
			.map(|pair| &pair[0]) // we have [last, next] and want the data for last
			.map(|placement| PlacementPageToken {
				id: placement.id as usize,
				timestamp: placement.timestamp as u32,
			})
			.map(|token| {
				let mut uri = format!(
					"/boards/{}/pixels?page={}&limit={}",
					board_id, token, limit,
				);

				// FIXME: urlencode
				if !filter.color.is_open() {
					uri.push_str(&format!("&color={}", filter.color))
				}
				if let Some(user) = filter.user {
					uri.push_str(&format!("&user={}", user))
				}
				if !filter.position.is_open() {
					uri.push_str(&format!("&position={}", filter.position))
				}
				if !filter.timestamp.is_open() {
					uri.push_str(&format!("&timestamp={}", filter.timestamp))
				}

				uri.parse().unwrap()
			});

		let mut items = Vec::with_capacity(limit);

		for placement in placements.into_iter().take(limit) {
			items.push(Placement {
				id: placement.id,
				position: placement.position as u64,
				color: placement.color as u8,
				timestamp: placement.timestamp as u32,
				user_id: placement.user_id,
				user: USER_ID_CACHE.get_uid(placement.user_id, &self.connection).await?,
			})
		}

		Ok(Page { items, next, previous: None })
	}

	pub async fn get_placement(
		&self,
		board_id: i32,
		position: u64,
	) -> DbResult<Option<Placement>> {
		let placement = placement::Entity::find()
			.filter(
				placement::Column::Board.eq(board_id)
					.and(placement::Column::Position.eq(position as i64)),
			)
			.order_by(placement::Column::Timestamp, sea_orm::Order::Desc)
			.order_by(placement::Column::Id, sea_orm::Order::Desc)
			.one(&self.connection).await?;
		
		if let Some(placement) = placement {
			Ok(Some(Placement {
				id: placement.id,
				position: placement.position as u64,
				color: placement.color as u8,
				timestamp: placement.timestamp as u32,
				user_id: placement.user_id,
				user: USER_ID_CACHE.get_uid(placement.user_id, &self.connection).await?,
			}))
		} else {
			Ok(None)
		}
	}

	pub async fn get_two_placements(
		&self,
		board_id: i32,
		position: u64,
	) -> DbResult<(Option<Placement>, Option<Placement>)> {
		let placements = placement::Entity::find()
			.filter(
				placement::Column::Board.eq(board_id)
					.and(placement::Column::Position.eq(position as i64)),
			)
			.order_by(placement::Column::Timestamp, sea_orm::Order::Desc)
			.order_by(placement::Column::Id, sea_orm::Order::Desc)
			.limit(2)
			.all(&self.connection).await?;

		let mut pair = Vec::with_capacity(2);
		for placement in placements {
			pair.push(Placement {
				id: placement.id,
				position: placement.position as u64,
				color: placement.color as u8,
				timestamp: placement.timestamp as u32,
				user_id: placement.user_id,
				user: USER_ID_CACHE.get_uid(placement.user_id, &self.connection).await?,
			})
		}
		let mut pair = pair.into_iter();
		Ok((pair.next(), pair.next()))
	}

	pub async fn delete_placement(&self, placement_id: i64,) -> DbResult<()> {
		placement::Entity::delete_by_id(placement_id)
			.exec(&self.connection).await?;
		Ok(())
	}

	pub async fn insert_placement(
		&self,
		board_id: i32,
		position: u64,
		color: u8,
		timestamp: u32,
		user_id: String,
	) -> DbResult<Placement> {
		placement::Entity::insert(
			placement::ActiveModel {
				id: NotSet,
				board: Set(board_id),
				position: Set(position as i64),
				color: Set(color as i16),
				timestamp: Set(timestamp as i32),
				user_id: Set(USER_ID_CACHE.get_id(user_id.clone(), &self.connection).await?),
			}
		)
		// TODO: try just returning the known data to skip the return for speed
		.exec_with_returning(&self.connection).await
		.map(|placement| Placement {
			id: placement.id,
			position: placement.position as u64,
			color: placement.color as u8,
			timestamp: placement.timestamp as u32,
			user_id: placement.user_id,
			user: user_id,
		})
		.map_err(BoardsDatabaseError::from)
	}

	pub async fn insert_placements(
		&self,
		board_id: i32,
		placements: &[(u64, u8)],
		timestamp: u32,
		user_id: String,
	) -> DbResult<Placement> {
		let uid = USER_ID_CACHE.get_id(user_id.clone(), &self.connection).await?;

		placement::Entity::insert_many(
			placements.iter().map(|(position, color)| {
				placement::ActiveModel {
					id: NotSet,
					board: Set(board_id),
					position: Set(*position as i64),
					color: Set(*color as i16),
					timestamp: Set(timestamp as i32),
					// TODO: this makes it clear that storing the user id as a
					// field is a *terrible* idea and it should be moved to it's
					// own table.
					user_id: Set(uid),
				}
			})
		)
		.exec_with_returning(&self.connection).await
		.map(|placement| Placement {
			id: placement.id,
			position: placement.position as u64,
			color: placement.color as u8,
			timestamp: placement.timestamp as u32,
			user_id: placement.user_id,
			user: user_id.clone(),
		})
		.map_err(BoardsDatabaseError::from)
	}

	/// use density buffer instead
	#[deprecated]
	pub async fn count_placements(
		&self,
		board_id: i32,
		position: u64,
		timestamp: u32,
	) -> DbResult<usize> {
		placement::Entity::find()
			.filter(
				placement::Column::Position.eq(position as i64)
				.and(placement::Column::Timestamp.lt(timestamp as i32))
				.and(placement::Column::Board.eq(board_id))
			)
			.count(&self.connection).await
			.map(|i| i as usize)
			.map_err(BoardsDatabaseError::from)
	}

	pub async fn list_user_placements(
		&self,
		board_id: i32,
		user_id: &str,
		limit: usize,
	) -> DbResult<Vec<Placement>> {
		let uid = USER_ID_CACHE.get_id(user_id.to_owned(), &self.connection).await?;
		let placements = placement::Entity::find()
			.filter(
				placement::Column::Board.eq(board_id)
					.and(placement::Column::UserId.eq(uid)),
			)
			.order_by(placement::Column::Timestamp, sea_orm::Order::Desc)
			.order_by(placement::Column::Id, sea_orm::Order::Desc)
			.limit(Some(limit as u64))
			.all(&self.connection).await?;

		Ok(placements.into_iter().rev().map(|placement| Placement {
			id: placement.id,
			position: placement.position as u64,
			color: placement.color as u8,
			timestamp: placement.timestamp as u32,
			user_id: placement.user_id,
			user: user_id.to_owned(),
		}).collect())
	}

	pub async fn user_count_between(
		&self,
		board_id: i32,
		min_time: i32,
		max_time: i32,
	) -> DbResult<usize> {
		placement::Entity::find()
			.distinct_on([placement::Column::UserId])
			.filter(placement::Column::Board.eq(board_id))
			.filter(placement::Column::Timestamp.between(min_time, max_time))
			.count(&self.connection).await
			.map(|count| count as usize)
			.map_err(BoardsDatabaseError::from)
	}

	pub async fn create_sector(
		&self,
		board_id: i32,
		index: i32,
		mask: Vec<u8>,
		initial: Vec<u8>,
	) -> DbResult<Sector> {

		let new_sector = board_sector::ActiveModel {
			board: Set(board_id),
			sector: Set(index),
			mask: Set(mask),
			initial: Set(initial),
		};

		let sector = board_sector::Entity::insert(new_sector)
			.exec_with_returning(&self.connection).await?;

		self.sector_from_model(sector).await
	}

	pub async fn get_sector(
		&self,
		board_id: i32,
		sector_index: i32,
	) -> DbResult<Option<Sector>> {
		let sector = board_sector::Entity::find_by_id((board_id, sector_index))
			.one(&self.connection).await?;

		match sector {
			Some(sector) => self.sector_from_model(sector).await.map(Some),
			None => Ok(None),
		}
	}

	async fn sector_from_model(
		&self,
		sector: board_sector::Model,
	) -> DbResult<Sector> {
		let index = sector.sector;
		let board = sector.board;
		let sector_size = sector.initial.len();

		let initial = BytesMut::from(&*sector.initial);
		let mask = BytesMut::from(&*sector.mask);
		let mut colors = initial.clone();
		let mut timestamps = BytesMut::from(&vec![0; sector_size * 4][..]);
		let mut density = BytesMut::from(&vec![0; sector_size * 4][..]);

		let start_position = sector_size as i64 * sector.sector as i64;
		let end_position = start_position + sector_size as i64 - 1;

		#[derive(Iden)]
		struct Inner;

		let column_timestamp_id_pair = Expr::tuple([
			Expr::col(placement::Column::Timestamp).into(),
			Expr::col(placement::Column::Id).into(),
		]);

		// TODO: look into storing this as indices on the database to skip
		// loading all placements.
		let placements = placement::Entity::find()
			.filter(placement::Column::Board.eq(board))
			.filter(placement::Column::Position.between(start_position, end_position))
			.order_by_asc(column_timestamp_id_pair)
			.all(&self.connection).await?;

		for placement in placements {
			let index = placement.position as usize;
			colors[index] = placement.color as u8;
			
			let index4 = index * 4..index * 4 + 4;
			let mut timestamp_slice = &mut timestamps[index4.clone()];
			timestamp_slice.put_u32_le(placement.timestamp as u32);

			let current_density = u32::from_le_bytes(unsafe {
				density[index4.clone()].try_into().unwrap_unchecked()
			});
			let mut density_slice = &mut density[index4];
			density_slice.put_u32_le(current_density + 1);
		}

		Ok(Sector {
			board,
			index,
			initial,
			mask,
			colors,
			timestamps,
			density,
		})
	}

	fn find_sector(board_id: i32, sector_index: i32) -> SimpleExpr {
		board_sector::Column::Sector
			.eq(sector_index)
			.and(board_sector::Column::Board.eq(board_id))
	}

	pub async fn write_sector_mask(
		&self,
		board_id: i32,
		sector_index: i32,
		mask: Vec<u8>,
	) -> DbResult<()> {
		board_sector::Entity::update_many()
			.col_expr(board_sector::Column::Mask, mask.into())
			.filter(Self::find_sector(board_id, sector_index))
			.exec(&self.connection).await
			.map(|_| ())
			.map_err(BoardsDatabaseError::from)
	}

	pub async fn write_sector_initial(
		&self,
		board_id: i32,
		sector_index: i32,
		initial: Vec<u8>,
	) -> DbResult<()> {
		board_sector::Entity::update_many()
			.col_expr(board_sector::Column::Initial, initial.into())
			.filter(Self::find_sector(board_id, sector_index))
			.exec(&self.connection).await
			.map(|_| ())
			.map_err(BoardsDatabaseError::from)
	}

	pub async fn list_notices(
		&self,
		token: NoticePageToken,
		limit: usize,
		filter: NoticeFilter,
	) -> DbResult<Page<Notice>> {
		let column_timestamp_id_pair = Expr::tuple([
			Expr::col(notice::Column::CreatedAt).into(),
			Expr::col(notice::Column::Id).into(),
		]);

		let value_timestamp_id_pair = Expr::tuple([
			(token.timestamp as i64).into(),
			(token.id as i32).into(),
		]);

		let notices = notice::Entity::find()
			.filter(Expr::gte(column_timestamp_id_pair.clone(), value_timestamp_id_pair))
			.apply_if(filter.author.as_ref(), |q, id| q.filter(notice::Column::Author.eq(id)))
			.apply_if(filter.content.as_ref(), |q, content| q.filter(notice::Column::Content.eq(content)))
			.apply_if(filter.title.as_ref(), |q, title| q.filter(notice::Column::Title.eq(title)))
			.apply_if(filter.created_at.start, |q, start| q.filter(notice::Column::CreatedAt.gte(start)))
			.apply_if(filter.created_at.end, |q, end| q.filter(notice::Column::CreatedAt.lte(end)))
			.apply_if(filter.expires_at.start, |q, start| q.filter(notice::Column::ExpiresAt.gte(start)))
			.apply_if(filter.expires_at.end, |q, end| q.filter(notice::Column::ExpiresAt.lte(end)))
			.order_by(column_timestamp_id_pair, sea_orm::Order::Asc)
			.limit(limit as u64 + 1)
			.all(&self.connection).await?;

		let next = notices.windows(2).nth(limit.saturating_sub(1))
			.map(|pair| &pair[0])
			.map(|notice| NoticePageToken {
				id: notice.id as _,
				timestamp: notice.created_at as _,
			})
			.map(|token| {
				format!(
					"/notices?page={}&limit={}",
					token, limit,
				).parse().unwrap()
			});

		let notices = notices.into_iter()
			.take(limit)
			.map(|notice| Notice {
				id: notice.id as usize,
				title: notice.title,
				content: notice.content,
				created_at: notice.created_at as _,
				expires_at: notice.expires_at.map(|v| v as _),
				author: notice.author,
			})
			.collect();
		
		Ok(Page { items: notices, next, previous: None })
	}

	pub async fn get_notice(&self, id: usize) -> DbResult<Option<Notice>> {
		notice::Entity::find_by_id(id as i32)
			.one(&self.connection).await
			.map(|n| n.map(|notice| Notice {
				id: notice.id as usize,
				title: notice.title,
				content: notice.content,
				created_at: notice.created_at as _,
				expires_at: notice.expires_at.map(|v| v as _),
				author: notice.author,
			}))
			.map_err(BoardsDatabaseError::from)
	}

	pub async fn create_notice(
		&self,
		title: String,
		content: String,
		expiry: Option<u64>,
	) -> DbResult<Notice> {
		let now = SystemTime::now()
			.duration_since(UNIX_EPOCH)
			.unwrap()
			.as_secs();

		notice::Entity::insert(notice::ActiveModel {
				id: NotSet,
				title: Set(title),
				content: Set(content),
				created_at: Set(now as _),
				expires_at: Set(expiry.map(|v| v as _)),
				author: NotSet, // TODO: set this
			})
			.exec_with_returning(&self.connection).await
			.map(|notice| Notice {
				id: notice.id as usize,
				title: notice.title,
				content: notice.content,
				created_at: notice.created_at as _,
				expires_at: notice.expires_at.map(|v| v as _),
				author: notice.author,
			})
			.map_err(BoardsDatabaseError::from)
	}

	pub async fn edit_notice(
		&self,
		id: usize,
		title: Option<String>,
		content: Option<String>,
		expiry: Option<Option<u64>>,
	) -> DbResult<Notice> {
		notice::Entity::update(notice::ActiveModel {
				id: Set(id as _),
				title: title.map(Set).unwrap_or(NotSet),
				content: content.map(Set).unwrap_or(NotSet),
				created_at: NotSet,
				expires_at: expiry.map(|e| Set(e.map(|v| v as _))).unwrap_or(NotSet),
				author: NotSet, // TODO: set this
			})
			.exec(&self.connection).await
			.map(|notice| Notice {
				id: notice.id as usize,
				title: notice.title,
				content: notice.content,
				created_at: notice.created_at as _,
				expires_at: notice.expires_at.map(|v| v as _),
				author: notice.author,
			})
			.map_err(BoardsDatabaseError::from)
	}

	// returns Ok(true) if the item was deleted or Ok(false) if it didn't exist
	pub async fn delete_notice(
		&self,
		id: usize,
	) -> DbResult<bool> {
		notice::Entity::delete_by_id(id as i32)
			.exec(&self.connection).await
			.map(|result| result.rows_affected == 1)
			.map_err(BoardsDatabaseError::from)
	}

	pub async fn list_board_notices(
		&self,
		board_id: i32,
		token: BoardsNoticePageToken,
		limit: usize,
		filter: BoardNoticeFilter,
	) -> DbResult<Page<BoardsNotice>> {
		let column_timestamp_id_pair = Expr::tuple([
			Expr::col(board_notice::Column::CreatedAt).into(),
			Expr::col(board_notice::Column::Id).into(),
		]);

		let value_timestamp_id_pair = Expr::tuple([
			(token.timestamp as i64).into(),
			(token.id as i32).into(),
		]);

		let notices = board_notice::Entity::find()
			.filter(board_notice::Column::Board.eq(board_id))
			.filter(Expr::gte(column_timestamp_id_pair.clone(), value_timestamp_id_pair))
			.apply_if(filter.author.as_ref(), |q, id| q.filter(board_notice::Column::Author.eq(id)))
			.apply_if(filter.content.as_ref(), |q, content| q.filter(board_notice::Column::Content.eq(content)))
			.apply_if(filter.title.as_ref(), |q, title| q.filter(board_notice::Column::Title.eq(title)))
			.apply_if(filter.created_at.start, |q, start| q.filter(board_notice::Column::CreatedAt.gte(start)))
			.apply_if(filter.created_at.end, |q, end| q.filter(board_notice::Column::CreatedAt.lte(end)))
			.apply_if(filter.expires_at.start, |q, start| q.filter(board_notice::Column::ExpiresAt.gte(start)))
			.apply_if(filter.expires_at.end, |q, end| q.filter(board_notice::Column::ExpiresAt.lte(end)))
			.order_by(column_timestamp_id_pair, sea_orm::Order::Asc)
			.limit(limit as u64 + 1)
			.all(&self.connection).await?;

		let next = notices.windows(2).nth(limit.saturating_sub(1))
			.map(|pair| &pair[0])
			.map(|notice| BoardsNoticePageToken {
				id: notice.id as _,
				timestamp: notice.created_at as _,
			})
			.map(|token| {
				format!(
					"/boards/{}/notices?page={}&limit={}",
					board_id, token, limit,
				).parse().unwrap()
			});

		let notices = notices.into_iter()
			.take(limit)
			.map(|notice| BoardsNotice {
				board: board_id as usize,
				id: notice.id as usize,
				title: notice.title,
				content: notice.content,
				created_at: notice.created_at as _,
				expires_at: notice.expires_at.map(|v| v as _),
				author: notice.author,
			})
			.collect();
		
		Ok(Page { items: notices, next, previous: None })
	}

	pub async fn get_board_notice(&self, board_id: i32, id: usize) -> DbResult<Option<BoardsNotice>> {
		board_notice::Entity::find_by_id(id as i32)
			.filter(board_notice::Column::Board.eq(board_id))
			.one(&self.connection).await
			.map(|n| n.map(|notice| BoardsNotice {
				board: board_id as usize,
				id: notice.id as usize,
				title: notice.title,
				content: notice.content,
				created_at: notice.created_at as _,
				expires_at: notice.expires_at.map(|v| v as _),
				author: notice.author,
			}))
			.map_err(BoardsDatabaseError::from)
	}

	pub async fn create_board_notice(
		&self,
		board_id: i32,
		title: String,
		content: String,
		expiry: Option<u64>,
	) -> DbResult<BoardsNotice> {
		let now = SystemTime::now()
			.duration_since(UNIX_EPOCH)
			.unwrap()
			.as_secs();

			board_notice::Entity::insert(board_notice::ActiveModel {
				id: NotSet,
				board: Set(board_id),
				title: Set(title),
				content: Set(content),
				created_at: Set(now as _),
				expires_at: Set(expiry.map(|v| v as _)),
				author: NotSet, // TODO: set this
			})
			.exec_with_returning(&self.connection).await
			.map(|notice| BoardsNotice {
				board: board_id as usize,
				id: notice.id as usize,
				title: notice.title,
				content: notice.content,
				created_at: notice.created_at as _,
				expires_at: notice.expires_at.map(|v| v as _),
				author: notice.author,
			})
			.map_err(BoardsDatabaseError::from)
	}

	pub async fn edit_board_notice(
		&self,
		board_id: i32,
		id: usize,
		title: Option<String>,
		content: Option<String>,
		expiry: Option<Option<u64>>,
	) -> DbResult<BoardsNotice> {
		board_notice::Entity::update(board_notice::ActiveModel {
				board: NotSet,
				id: Set(id as _),
				title: title.map(Set).unwrap_or(NotSet),
				content: content.map(Set).unwrap_or(NotSet),
				created_at: NotSet,
				expires_at: expiry.map(|e| Set(e.map(|v| v as _))).unwrap_or(NotSet),
				author: NotSet, // TODO: set this
			})
			.filter(board_notice::Column::Board.eq(board_id))
			.exec(&self.connection).await
			.map(|notice| BoardsNotice {
				board: board_id as usize,
				id: notice.id as usize,
				title: notice.title,
				content: notice.content,
				created_at: notice.created_at as _,
				expires_at: notice.expires_at.map(|v| v as _),
				author: notice.author,
			})
			.map_err(BoardsDatabaseError::from)
	}

	// returns Ok(true) if the item was deleted or Ok(false) if it didn't exist
	pub async fn delete_board_notice(
		&self,
		board_id: i32,
		id: usize,
	) -> DbResult<bool> {
		board_notice::Entity::delete_by_id(id as i32)
			.filter(board_notice::Column::Board.eq(board_id))
			.exec(&self.connection).await
			.map(|result| result.rows_affected == 1)
			.map_err(BoardsDatabaseError::from)
	}
}