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
	database::Pool,
	filters::{
		body::patch,
		header::{
			authorization,
			range::{self, Range},
		},
		resource::{board, database},
	},
	objects::*,
};

pub mod auth;
pub mod core;
