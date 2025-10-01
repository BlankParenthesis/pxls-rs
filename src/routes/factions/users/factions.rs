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
use crate::database::{Database, DbConn, Specifier, User, UserFactionMemberListSpecifier};
use super::super::factions::FactionFilter;

pub fn list(
	db: Arc<Database>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	let permissions = Permission::UsersGet | Permission::UsersFactionsList;

	UserFactionMemberListSpecifier::path()
		.and(warp::get())
		.and(warp::query())
		.and(warp::query())
		.and(authorization::permissions(db))
		.and_then(move |list: UserFactionMemberListSpecifier, pagination, filter, user_permissions, requester: Option<User>, connection| async move {
			let is_current_user = requester.as_ref()
				.map(|u| *u.specifier() == list.user())
				.unwrap_or(false);

			let check = if is_current_user {
				authorization::has_permissions_current
			} else {
				authorization::has_permissions
			};

			if check(user_permissions, permissions) {
				Ok((list, pagination, filter, connection))
			} else {
				Err(warp::reject::custom(PermissionsError::MissingPermission))
			}
		})
		.untuple_one()
		.then(move |list: UserFactionMemberListSpecifier, pagination: PaginationOptions<_>, filter: FactionFilter, connection: DbConn| async move {
			let page = pagination.page;
			let limit = pagination.limit
				.unwrap_or(CONFIG.default_page_item_limit)
				.clamp(1, CONFIG.max_page_item_limit);
			
			connection.list_user_factions(&list, page, limit, filter).await
				.map(|page| warp::reply::json(&page))
		})
}
