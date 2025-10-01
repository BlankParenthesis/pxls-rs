use std::fmt;

use sea_orm::sea_query::{Expr, SimpleExpr};
use sea_orm::{ActiveValue::NotSet, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect, QueryTrait, Set, StreamTrait, TransactionTrait};
use serde::de;
use serde::de::Visitor;
use serde::{Deserialize, Deserializer, Serialize};
use url::form_urlencoded::byte_serialize;

use crate::board::PendingPlacement;
use crate::filter::response::paginated_list::{Page, PageToken};
use crate::filter::response::reference::Reference;
use crate::routes::core::boards::pixels::PlacementFilter;

use super::entities::*;
use super::user::{User, UserSpecifier};
use super::board::BoardSpecifier;
use super::{Order, Connection, DbResult, DatabaseError};
use super::specifier::{SpecifierParser, Id, SpecfierParseError, Specifier, PathPart, specifier_path};

#[derive(Debug, Clone, Copy)]
pub struct PlacementListSpecifier {
	board: i32,
}

impl PlacementListSpecifier {
	pub fn board(&self) -> BoardSpecifier {
		BoardSpecifier(self.board)
	}
}

impl Specifier for PlacementListSpecifier {
	fn filter(&self) -> SimpleExpr {
		placement::Column::Board.eq(self.board)
	}
	
	fn from_ids(ids: &[&str]) -> Result<Self, SpecfierParseError> {
		let board = ids[0].parse()?;
		Ok(Self { board })
	}
	
	fn to_ids(&self) -> Box<[Id]> {
		Box::new([Id::I32(self.board)])
	}
	
	fn parts() -> &'static [PathPart] {
		specifier_path!("boards", board, "pixels")
	}
}

impl From<BoardSpecifier> for PlacementListSpecifier {
	fn from(value: BoardSpecifier) -> Self {
		Self { board: value.0 }
	}
}

impl<'de> Deserialize<'de> for PlacementListSpecifier {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where D: serde::Deserializer<'de> {
		deserializer.deserialize_str(SpecifierParser::new("A placement list uri"))
	}
}

impl Serialize for PlacementListSpecifier {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where S: serde::Serializer {
		serializer.serialize_str(self.to_uri().path())
	}
}

#[derive(Debug, Clone, Copy)]
pub struct PlacementSpecifier {
	board: i32,
	position: u64,
}

impl PlacementSpecifier {
	pub fn board(&self) -> BoardSpecifier {
		BoardSpecifier(self.board)
	}
	
	pub fn position(&self) -> u64 {
		self.position
	}
}

impl Specifier for PlacementSpecifier {
	fn filter(&self) -> SimpleExpr {
		placement::Column::Board.eq(self.board)
			.and(placement::Column::Position.eq(self.position))
	}
	
	fn from_ids(ids: &[&str]) -> Result<Self, SpecfierParseError> {
		let board = ids[0].parse()?;
		let position = ids[1].parse()?;
		Ok(Self { board, position })
	}
	
	fn to_ids(&self) -> Box<[Id]> {
		Box::new([Id::I32(self.board), Id::U64(self.position)])
	}
	
	fn parts() -> &'static [PathPart] {
		specifier_path!("boards", board, "pixels", position)
	}
}

impl<'de> Deserialize<'de> for PlacementSpecifier {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where D: serde::Deserializer<'de> {
		deserializer.deserialize_str(SpecifierParser::new("A placement uri"))
	}
}

impl Serialize for PlacementSpecifier {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where S: serde::Serializer {
		serializer.serialize_str(self.to_uri().path())
	}
}

pub struct PlacementPageToken {
	pub id: usize,
	pub timestamp: u32,
}

impl PageToken for PlacementPageToken {
	fn start() -> Self {
		Self { id: 0, timestamp: 0 }
	}
}

impl Default for PlacementPageToken {
	fn default() -> Self { Self::start() }
}

impl fmt::Display for PlacementPageToken {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{}_{}", self.timestamp, self.id)
	}
}

impl<'de> Deserialize<'de> for PlacementPageToken {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where D: Deserializer<'de> {
		struct PageVisitor;

		impl<'de> Visitor<'de> for PageVisitor {
			type Value = PlacementPageToken;

			fn expecting(
				&self,
				formatter: &mut fmt::Formatter,
			) -> fmt::Result {
				formatter.write_str("a string of two integers, separated by an underscore")
			}

			fn visit_str<E>(
				self,
				value: &str,
			) -> Result<Self::Value, E> where E: de::Error {
				let (timestamp, id) = value.split_once('_')
					.ok_or_else(|| E::custom("missing underscore"))?;
				
				Ok(PlacementPageToken {
					id: id.parse()
						.map_err(|_| E::custom("id invalid"))?,
					timestamp: timestamp.parse()
						.map_err(|_| E::custom("timestamp invalid"))?,
				})
			}
		}

		deserializer.deserialize_str(PageVisitor)
	}
}

#[derive(Debug, Serialize, Clone)]
pub struct Placement {
	pub position: u64,
	pub color: u8,
	pub modified: u32,
	pub user: Reference<User>,
}

#[derive(Debug, Clone, Copy)]
pub struct LastPlacement {
	pub id: i64,
	pub modified: u32,
	pub color: u8,
	pub user: UserSpecifier,
}

#[derive(Debug, Clone, Copy)]
pub struct CachedPlacement {
	pub modified: u32,
	pub user: UserSpecifier,
}

impl From<placement::Model> for CachedPlacement {
	fn from(value: placement::Model) -> Self {
		Self {
			modified: value.timestamp as _,
			user: UserSpecifier(value.user_id),
		}
	}
}

impl<C: TransactionTrait + ConnectionTrait + StreamTrait> Connection<C> {
	
	pub async fn list_placements(
		&self,
		list: &PlacementListSpecifier,
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
			.find_also_related(user::Entity)
			.filter(list.filter())
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
			.map(|(placement, _)| PlacementPageToken {
				id: placement.id as usize,
				timestamp: placement.timestamp as u32,
			})
			.map(|token| {
				let path_uri = list.to_uri();
				let path = path_uri.path();
				let mut uri = format!("{path}?page={token}&limit={limit}");

				if !filter.color.is_open() {
					uri.push_str(&format!("&color={}", filter.color))
				}
				if let Some(user) = filter.user {
					if let Some(user) = byte_serialize(user.as_bytes()).next() {
						uri.push_str(&format!("&user={}", user))
					}
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

		for (placement, user) in placements.into_iter().take(limit) {
			let user = user.unwrap();
			items.push(Placement {
				position: placement.position as u64,
				color: placement.color as u8,
				modified: placement.timestamp as u32,
				user: Reference::from(User::from(user)),
			})
		}

		Ok(Page { items, next, previous: None })
	}

	pub async fn get_placement(
		&self,
		placement: &PlacementSpecifier,
	) -> DbResult<Option<Placement>> {
		let placement = placement::Entity::find()
			.find_also_related(user::Entity)
			.filter(placement.filter())
			.order_by(placement::Column::Timestamp, sea_orm::Order::Desc)
			.order_by(placement::Column::Id, sea_orm::Order::Desc)
			.one(&self.connection).await?;
		
		if let Some((placement, user)) = placement {
			let user = user.unwrap();
			Ok(Some(Placement {
				position: placement.position as u64,
				color: placement.color as u8,
				modified: placement.timestamp as u32,
				user: Reference::from(User::from(user)),
			}))
		} else {
			Ok(None)
		}
	}
	
	pub async fn get_two_placements(
		&self,
		placement: &PlacementSpecifier,
	) -> DbResult<(Option<LastPlacement>, Option<LastPlacement>)> {
		let placements = placement::Entity::find()
			.filter(placement.filter())
			.order_by(placement::Column::Timestamp, sea_orm::Order::Desc)
			.order_by(placement::Column::Id, sea_orm::Order::Desc)
			.limit(2)
			.all(&self.connection).await?;

		let mut pair = Vec::with_capacity(2);
		for placement in placements {
			let placement = LastPlacement {
				id: placement.id,
				modified: placement.timestamp as _,
				color: placement.color as _,
				user: UserSpecifier(placement.user_id),
			};
			pair.push(placement)
		}
		let mut pair = pair.into_iter();
		Ok((pair.next(), pair.next()))
	}
	
	pub async fn create_placements(
		&self,
		list: PlacementListSpecifier,
		placements: &[PendingPlacement],
	) -> DbResult<()> {
		placement::Entity::insert_many(
			placements.iter().map(|p| {
				placement::ActiveModel {
					id: NotSet,
					board: Set(list.board),
					position: Set(p.position as i64),
					color: Set(p.color as i16),
					timestamp: Set(p.timestamp as i32),
					user_id: Set(p.user.0),
				}
			})
		)
		.exec(&self.connection).await
		.map(|_| ())
		.map_err(DatabaseError::from)
	}
	
	pub async fn delete_placement(&self, placement_id: i64,) -> DbResult<()> {
		placement::Entity::delete_by_id(placement_id)
			.exec(&self.connection).await?;
		Ok(())
	}
}
