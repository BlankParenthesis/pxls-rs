use http::{Response, StatusCode};
use serde::Serialize;
use url::Url;
use warp::{
	reject::Rejection,
	reply::{self, json, Reply},
	Filter,
};

use crate::{
	access::permissions::{with_permission, Permission},
	filters::{
		header::{
			authorization,
			range::{self, Range},
		},
		resource::{board, database},
	},
	objects::*,
};

pub mod authentication;
pub mod core;
pub mod board_data_initial;
pub mod board_data_mask;
pub mod board_data_timestamps;
pub mod board_lifecycle;
