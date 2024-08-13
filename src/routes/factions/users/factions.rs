use std::sync::Arc;

use warp::{
	Filter,
	Reply,
	Rejection,
};

use crate::filter::header::authorization::{self, Bearer, PermissionsError};
use crate::filter::response::paginated_list::{
	PaginationOptions,
	MAX_PAGE_ITEM_LIMIT,
	DEFAULT_PAGE_ITEM_LIMIT,
};

use crate::permissions::Permission;
use crate::database::{UsersDatabase, UsersConnection, LdapPageToken};
use super::super::factions::FactionFilter;

pub fn list(
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	let permissions = Permission::UsersGet | Permission::UsersFactionsList;

	warp::path("users")
		.and(warp::path::param())
		.and(warp::path("factions"))
		.and(warp::path::end())
		.and(warp::get())
		.and(warp::query())
		.and(warp::query())
		.and(authorization::permissions(users_db))
		.and_then(move |uid: String, pagination, filter, user_permissions, bearer: Option<Bearer>, connection| async move {
			let is_current_user = bearer.as_ref()
				.map(|bearer| bearer.id == uid)
				.unwrap_or(false);

			let check = if is_current_user {
				authorization::has_permissions_current
			} else {
				authorization::has_permissions
			};


			if check(user_permissions, permissions) {
				Ok((uid, pagination, filter, connection))
			} else {
				Err(warp::reject::custom(PermissionsError::MissingPermission))
			}
		})
		.untuple_one()
		.then(move |uid: String, pagination: PaginationOptions<LdapPageToken>, filter: FactionFilter, mut connection: UsersConnection| async move {
			let page = pagination.page;
			let limit = pagination.limit
				.unwrap_or(DEFAULT_PAGE_ITEM_LIMIT)
				.clamp(1, MAX_PAGE_ITEM_LIMIT);
			
			connection.list_user_factions(page, limit, filter, &uid).await
				.map(|page| warp::reply::json(&page))
		})
}