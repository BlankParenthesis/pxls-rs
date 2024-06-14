use std::{sync::Arc, collections::HashMap};

use serde::Serialize;
use warp::{
	Filter,
	Reply,
	Rejection,
};

use crate::routes::users::users::UserFilter;
use crate::database::DatabaseError;
use crate::filter::response::paginated_list::{
	Page,
	PaginationOptions,
	DEFAULT_PAGE_ITEM_LIMIT,
	MAX_PAGE_ITEM_LIMIT
};
use crate::filter::header::authorization::{self, Bearer, PermissionsError};
use crate::filter::response::reference::Reference;
use crate::filter::resource::database;
use crate::permissions::Permission;
use crate::database::{UsersDatabase, UsersConnection, LdapPageToken, User, BoardsDatabase, BoardsConnection};

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

pub async fn calculate_stats(
	uid: String,
	boards: &crate::BoardDataMap,
	boards_connection: &BoardsConnection,
	users_connection: &mut UsersConnection,
) -> Result<UserStats, DatabaseError> {
	let user = users_connection.get_user(&uid).await?;
	let user = Reference::new(User::uri(&uid), user);

	let board_list = boards.read().await;

	let mut totals = PlacementStatistics::default();
	let mut boards = vec![];

	for (id, board) in &*board_list {
		let board = board.read().await;
		let board = board.as_ref().expect("board went missing");
		let stats = board.user_stats(&uid, boards_connection).await?;

		if stats.colors.is_empty() {
			continue;
		}
		
		for count in stats.colors.values() {
			totals.placed += count.placed;
		}

		let board_uri = format!("/boards/{}", id).parse().unwrap();
		let board = Reference::new(board_uri, ());
		boards.push(BoardStats { board, stats });
	}

	Ok(UserStats { user, boards, totals })
}


pub fn list(
	boards: crate::BoardDataMap,
	users_db: Arc<UsersDatabase>,
	boards_db: Arc<BoardsDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("users")
		.and(warp::path("stats"))
		.and(warp::path::end())
		.and(warp::get())
		.and(warp::query())
		.and(warp::query())
		.and(authorization::authorized(users_db, Permission::UsersStatsList.into()))
		.and(database::connection(boards_db))
		.then(move |pagination: PaginationOptions<LdapPageToken>, filter: UserFilter, _, mut users_connection: UsersConnection, boards_connection| {
			let boards = boards.clone();
			async move {
				let page = pagination.page;
				let limit = pagination.limit
					.unwrap_or(DEFAULT_PAGE_ITEM_LIMIT)
					.clamp(1, MAX_PAGE_ITEM_LIMIT); // TODO: maybe raise upper limit
				
				let page = users_connection.list_users(page, limit, filter).await?;
				let next = page.next.map(|n| {
					format!("/users/stats?{}", n.query().unwrap_or("")).parse().unwrap()
				});
				let previous = page.previous.map(|p| {
					format!("/users/stats?{}", p.query().unwrap_or("")).parse().unwrap()
				});
				let users = page.items;

				let mut stats = Vec::with_capacity(users.len());
				for user in users {
					let uid = user.uri.path().split('/').last().unwrap().to_owned();
					let stat = calculate_stats(
						uid,
						&boards,
						&boards_connection,
						&mut users_connection,
					).await?;
					stats.push(stat);
				}

				let page = Page { items: stats, next, previous };
				Ok::<_, DatabaseError>(warp::reply::json(&page))
			}
		})
}


pub fn get(
	boards: crate::BoardDataMap,
	users_db: Arc<UsersDatabase>,
	boards_db: Arc<BoardsDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	let permissions = Permission::UsersStatsGet.into();

	warp::path("users")
		.and(warp::path::param())
		.and(warp::path::path("stats"))
		.and(warp::path::end())
		.and(warp::get())
		.and(authorization::permissions(users_db))
		.and_then(move |uid: String, user_permissions, bearer: Option<Bearer>, connection| async move {
			let is_current_user = bearer.as_ref()
				.map(|bearer| bearer.id == uid)
				.unwrap_or(false);

			let check = if is_current_user {
				authorization::has_permissions_current
			} else {
				authorization::has_permissions
			};

			if check(user_permissions, permissions) {
				Ok((uid, connection))
			} else {
				Err(warp::reject::custom(PermissionsError::MissingPermission))
			}
		})
		.untuple_one()
		.and(database::connection(boards_db))
		.then(move |uid: String, mut users_connection, boards_connection| {
			let boards = boards.clone();
			async move {
				calculate_stats(
					uid,
					&boards,
					&boards_connection,
					&mut users_connection,
				).await.map(|stats| warp::reply::json(&stats))
			}
		})
}