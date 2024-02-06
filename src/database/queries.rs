use sea_orm::{ConnectionTrait, EntityTrait};

use crate::board::board::Board;

use super::boards::{entities::board, DbResult};

use ldap3::{
	SearchEntry,
	Scope,
	controls::{PagedResults, Control, ControlType},
	ldap_escape,
};
use crate::config::CONFIG;
use base64::prelude::*;
use super::users::{Connection, User, UserFetchError};

pub async fn load_boards<Connection: ConnectionTrait>(
	connection: &Connection
) -> DbResult<Vec<Board>> {
	let db_boards = board::Entity::find()
		.all(connection).await?;

	let mut boards = Vec::with_capacity(db_boards.len());

	for board in db_boards {
		boards.push(Board::load(board, connection).await?);
	}
	
	Ok(boards)
}

pub async fn list_users(
	connection: &mut Connection,
	page: Option<String>,
	limit: usize,
) -> Result<(Option<String>, Vec<User>), UserFetchError> {
	let pager = PagedResults {
		size: limit as i32,
		cookie: page.map(|p| BASE64_URL_SAFE.decode(p))
			.unwrap_or(Ok(vec![]))
			.map_err(|_| UserFetchError::InvalidPage)?,
	};

	let filter = format!("({}=*)", CONFIG.users_ldap_id_field);
	let (results, status) = connection.with_controls(pager)
		.search(
			&CONFIG.users_ldap_base,
			Scope::OneLevel,
			filter.as_str(),
			[
				&CONFIG.users_ldap_id_field,
				&CONFIG.users_ldap_username_field,
				"createTimestamp"
			],
		).await
		.map_err(UserFetchError::LdapError)?
		.success()
		// a bit presumptuous, but should be correct enough
		.map_err(|_| UserFetchError::InvalidPage)?;

	let page_data = status.ctrls.iter()
		.find(|Control(t, _)| matches!(t, Some(ControlType::PagedResults)))
		.map(|Control(_, d)| d.parse::<PagedResults>())
		.ok_or(UserFetchError::MissingPagerData)?;

	let items = results.into_iter()
		.map(SearchEntry::construct)
		.map(User::try_from)
		.map(|r| r.map_err(UserFetchError::ParseError))
		.collect::<Result<_, _>>()?;
	
	if page_data.cookie.is_empty() {
		Ok((None, items))
	} else {
		let page = BASE64_URL_SAFE.encode(page_data.cookie);
		Ok((Some(page), items))
	}
}

pub async fn get_user(
	connection: &mut Connection,
	uid: String,
) -> Result<User, UserFetchError> {
	let filter = format!("({}={})", CONFIG.users_ldap_id_field, ldap_escape(uid));
	let (results, _) = connection
		.search(
			&CONFIG.users_ldap_base,
			Scope::OneLevel,
			filter.as_str(),
			[
				&CONFIG.users_ldap_id_field,
				&CONFIG.users_ldap_username_field,
				"createTimestamp"
			],
		).await
		.map_err(UserFetchError::LdapError)?
		.success()
		.map_err(UserFetchError::LdapError)?;

	match results.len() {
		0 => Err(UserFetchError::MissingUser),
		1 => {
			let result = results.into_iter()
				.next()
				.map(SearchEntry::construct)
				.unwrap();

			User::try_from(result)
				.map_err(UserFetchError::ParseError)
		},
		_ => Err(UserFetchError::AmbiguousUser),
	}
}