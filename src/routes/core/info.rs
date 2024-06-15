use std::sync::Arc;

use serde::Serialize;
use serde_with::skip_serializing_none;
use warp::{Filter, Reply, Rejection};

use crate::filter::header::authorization::authorized;
use crate::permissions::Permission;
use crate::database::UsersDatabase;

#[skip_serializing_none]
#[derive(Serialize)]
pub struct ServerInfo {
	name: Option<&'static str>,
	version: Option<&'static str>,
	source: Option<&'static str>,
	extensions: &'static [&'static str],
}

lazy_static! {
	static ref SERVER_INFO: ServerInfo = ServerInfo {
		// TODO: think of a good name. ideas:
		// iridescence / pearlescence

		// Using the pxls name seems bit presumptions given this shares
		// basically nothing with original pxls, but pxls-based names could be:
		// pxls-rs
		// pxls 2
		// neo-pxls

		name: Some("unnamed-newpxls-rs"),
		version: option_env!("CARGO_PKG_VERSION").filter(|s| !s.is_empty()),
		source: option_env!("CARGO_PKG_REPOSITORY").filter(|s| !s.is_empty()),
		extensions: &[
			"authentication",
			"board_timestamps",
			"board_mask",
			"board_initial",
			"board_lifecycle",
			"users",
			"roles",
			"user_count",
			"board_moderation",
			"undo",
			"factions",
			"site_notices",
			"board_notices",
			"list_filtering",
			"reports",
			"placement_statistics",
			"user_bans",
		],
	};
}

pub fn get(
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("info")
		.and(warp::path::end())
		.and(warp::get())
		.and(authorized(users_db, Permission::Info.into()))
		.map(|_, _| warp::reply::json(&*SERVER_INFO))
}
