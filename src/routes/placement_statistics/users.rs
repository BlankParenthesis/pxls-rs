use std::{sync::Arc, collections::HashMap};

use reqwest::StatusCode;
use serde::Serialize;
use warp::{
	Filter,
	Reply,
	Rejection,
};

use crate::config::CONFIG;
use crate::routes::users::users::UserFilter;
use crate::database::{DatabaseError, Specifier, UserStatsSpecifier};
use crate::filter::response::paginated_list::{
	Page,
	PaginationOptions,
};
use crate::filter::header::authorization::{self, PermissionsError};
use crate::filter::response::reference::Reference;
use crate::permissions::Permission;
use crate::database::{User, Database, DbConn};

#[derive(Serialize)]
pub struct UserStats {
	pub user: Reference<User>,
	pub totals: PlacementStatistics,
	pub boards: Vec<BoardStats>,
}

#[derive(serde::Serialize)]
pub struct BoardStats {
	pub board: Reference<()>,
	pub stats: PlacementColorStatistics,
}

#[derive(Serialize, Debug, Default, Clone, Copy)]
pub struct PlacementStatistics {
	pub placed: usize,
}
#[derive(Serialize, Debug, Default, Clone)]
pub struct PlacementColorStatistics {
	pub colors: HashMap<u8, PlacementStatistics>,
}

// TODO: this function seems very off, it should probably go somewhere else or
// at least use a more sensible locking scheme to prevent double read locks
pub async fn calculate_stats(
	user: &User,
	boards: &crate::BoardDataMap,
) -> Result<UserStats, DatabaseError> {
	let board_list = boards.read().await;

	let mut totals = PlacementStatistics::default();
	let mut boards = vec![];

	for (id, board) in &*board_list {
		let board = board.read().await;
		let board = board.as_ref().expect("board went missing");
		let stats = board.user_stats(user).await?;

		if stats.colors.is_empty() {
			continue;
		}
		
		for count in stats.colors.values() {
			totals.placed += count.placed;
		}

		let board = Reference::new_empty(id.to_uri());
		boards.push(BoardStats { board, stats });
	}
	
	let user = Reference::from(user.clone());

	Ok(UserStats { user, boards, totals })
}


pub fn list(
	boards: crate::BoardDataMap,
	db: Arc<Database>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("users")
		.and(warp::path("stats"))
		.and(warp::path::end())
		.and(warp::get())
		.and(warp::query())
		.and(warp::query())
		.and(authorization::authorized(db, Permission::UsersStatsList.into()))
		.then(move |pagination: PaginationOptions<_>, filter: UserFilter, _, connection: DbConn| {
			let boards = boards.clone();
			async move {
				let page = pagination.page;
				let limit = pagination.limit
					.unwrap_or(CONFIG.default_page_item_limit)
					.clamp(1, CONFIG.max_page_item_limit);
				
				let page = connection.list_users(page, limit, filter).await?;
				let next = page.next.map(|n| {
					format!("/users/stats?{}", n.query().unwrap_or("")).parse().unwrap()
				});
				let previous = page.previous.map(|p| {
					format!("/users/stats?{}", p.query().unwrap_or("")).parse().unwrap()
				});
				let users = page.items;

				let mut stats = Vec::with_capacity(users.len());
				for user in users {
					let stat = calculate_stats(&user.view, &boards).await?;
					stats.push(stat);
				}

				let page = Page { items: stats, next, previous };
				Ok::<_, StatusCode>(warp::reply::json(&page))
			}
		})
}


pub fn get(
	boards: crate::BoardDataMap,
	db: Arc<Database>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	let permissions = Permission::UsersStatsGet.into();

	UserStatsSpecifier::path()
		.and(warp::get())
		.and(authorization::permissions(db))
		.and_then(move |stats: UserStatsSpecifier, user_permissions, requester: Option<User>, connection| async move {
			let is_current_user = requester.as_ref()
				.map(|u| *u.specifier() == stats.user())
				.unwrap_or(false);

			let check = if is_current_user {
				authorization::has_permissions_current
			} else {
				authorization::has_permissions
			};

			if check(user_permissions, permissions) {
				Ok((stats, connection))
			} else {
				Err(warp::reject::custom(PermissionsError::MissingPermission))
			}
		})
		.untuple_one()
		.then(move |stats: UserStatsSpecifier, connection: DbConn| {
			let boards = boards.clone();
			async move {
				let user = connection.get_user(&stats.user()).await?
					.ok_or(StatusCode::NOT_FOUND)?;
				calculate_stats(&user, &boards).await
					.map(|stats| warp::reply::json(&stats))
					.map_err(StatusCode::from)
			}
		})
}
