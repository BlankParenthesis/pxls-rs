use std::sync::Arc;

use warp::{
	Filter,
	Reply,
	Rejection,
};

use crate::config::CONFIG;
use crate::filter::header::authorization::{self, PermissionsError};
use crate::filter::response::paginated_list::PaginationOptions;

use crate::permissions::Permission;
use crate::database::{BoardsConnection, BoardsDatabase, User, UserSpecifier};
use super::super::factions::FactionFilter;

pub fn list(
	db: Arc<BoardsDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	let permissions = Permission::UsersGet | Permission::UsersFactionsList;

	warp::path("users")
		.and(warp::path::param())
		.and(warp::path("factions"))
		.and(warp::path::end())
		.and(warp::get())
		.and(warp::query())
		.and(warp::query())
		.and(authorization::permissions(db))
		.and_then(move |user: UserSpecifier, pagination, filter, user_permissions, requester: Option<User>, connection| async move {
			let is_current_user = requester.as_ref()
				.map(|u| u.specifier() == user)
				.unwrap_or(false);

			let check = if is_current_user {
				authorization::has_permissions_current
			} else {
				authorization::has_permissions
			};

			if check(user_permissions, permissions) {
				Ok((user, pagination, filter, connection))
			} else {
				Err(warp::reject::custom(PermissionsError::MissingPermission))
			}
		})
		.untuple_one()
		.then(move |user: UserSpecifier, pagination: PaginationOptions<_>, filter: FactionFilter, connection: BoardsConnection| async move {
			let page = pagination.page;
			let limit = pagination.limit
				.unwrap_or(CONFIG.default_page_item_limit)
				.clamp(1, CONFIG.max_page_item_limit);
			
			connection.list_user_factions(&user, page, limit, filter).await
				.map(|page| warp::reply::json(&page))
		})
}
