use std::time::{Duration, SystemTime, UNIX_EPOCH};

use bytes::{BytesMut, BufMut};
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
	sea_query::{Expr, SimpleExpr, Query, self},
	QuerySelect,
	QueryOrder,
	PaginatorTrait,
	Iden,
	ConnectionTrait,
};
use sea_orm_migration::MigratorTrait;

use crate::{config::CONFIG, filter::response::paginated_list::PageToken};
use crate::board::{Palette, Color, Board, Placement, Sector};

mod entities;

use entities::*;
use migration::Migrator;

use super::Order;

pub type DbResult<T> = Result<T, sea_orm::DbErr>;

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

pub struct BoardsConnection<Connection: TransactionTrait + ConnectionTrait> {
	connection: Connection,
}

impl BoardsConnection<DatabaseTransaction> {
	pub async fn commit(self) -> DbResult<()> {
		self.connection.commit().await
	}
}

impl<C: TransactionTrait + ConnectionTrait> BoardsConnection<C> {
	pub async fn begin(&self) -> DbResult<BoardsConnection<DatabaseTransaction>> {
		self.connection.begin().await
			.map(|connection| BoardsConnection { connection })
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
					value: color.value as u32
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
	
		for (index, Color { name, value }) in palette {
			let color = color::ActiveModel {
				board: Set(board_id),
				index: Set(index as i32),
				name: Set(name.clone()),
				value: Set(value as i32),
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
	}

	pub async fn list_placements(
		&self,
		board_id: i32,
		token: PageToken,
		limit: usize,
		order: Order,
	) -> DbResult<(Option<PageToken>, Vec<Placement>)> {
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
			Order::Forward => Expr::lt(compare_lhs, compare_rhs),
			Order::Reverse => Expr::gte(compare_lhs, compare_rhs),
		};

		let order = match order {
			Order::Forward => sea_orm::Order::Asc,
			Order::Reverse => sea_orm::Order::Desc,
		};

		let placements = placement::Entity::find()
			.filter(placement::Column::Board.eq(board_id))
			.filter(compare)
			.order_by(column_timestamp_id_pair, order)
			.limit(limit as u64)
			.all(&self.connection).await?;

		let token = placements.last().map(|placement| PageToken {
			id: placement.id as usize,
			timestamp: placement.timestamp as u32,
		});

		let placements = placements.into_iter()
			.map(|placement| Placement {
				position: placement.position as u64,
				color: placement.color as u8,
				timestamp: placement.timestamp as u32,
			})
			.collect();
		
		Ok((token, placements))
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

		Ok(placement.map(|placement| Placement {
			position: placement.position as u64,
			color: placement.color as u8,
			timestamp: placement.timestamp as u32,
		}))
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
				user_id: Set(user_id),
			}
		)
		.exec_with_returning(&self.connection).await
		.map(|placement| Placement {
			position: placement.position as u64,
			color: placement.color as u8,
			timestamp: placement.timestamp as u32,
		})
	}

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
	}

	pub async fn list_user_placements(
		&self,
		board_id: i32,
		user_id: &str,
		limit: usize,
	) -> DbResult<Vec<Placement>> {
		let placements = placement::Entity::find()
			.filter(
				placement::Column::Board.eq(board_id)
					.and(placement::Column::UserId.eq(user_id.clone())),
			)
			.order_by(placement::Column::Timestamp, sea_orm::Order::Desc)
			.order_by(placement::Column::Id, sea_orm::Order::Desc)
			.limit(Some(limit as u64))
			.all(&self.connection).await?;

		Ok(placements.into_iter().rev().map(|placement| Placement {
			position: placement.position as u64,
			color: placement.color as u8,
			timestamp: placement.timestamp as u32,
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

		let start_position = sector_size as i64 * sector.sector as i64;
		let end_position = start_position + sector_size as i64 - 1;

		#[derive(Iden)]
		struct Inner;

		let placements = placement::Entity::find()
			.filter(placement::Column::Board.eq(board))
			.filter(placement::Column::Position.between(start_position, end_position))
			.filter(placement::Column::Id.in_subquery(
				Query::select()
					.from_as(placement::Entity, Inner)
					.column((Inner, placement::Column::Id))
					.and_where(
						Expr::col((placement::Entity, placement::Column::Position))
							.equals((Inner, placement::Column::Position))
					)
					.order_by((Inner, placement::Column::Timestamp), sea_orm::Order::Desc)
					.order_by((Inner, placement::Column::Id), sea_orm::Order::Desc)
					.limit(1)
					.to_owned()
			))
			.all(&self.connection).await?;

		for placement in placements {
			let index = placement.position as usize;
			colors[index] = placement.color as u8;
			let mut timestamp_slice = &mut timestamps[index * 4..index * 4 + 4];
			timestamp_slice.put_u32_le(placement.timestamp as u32);
		}

		Ok(Sector {
			board,
			index,
			initial,
			mask,
			colors,
			timestamps,
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
	}
}