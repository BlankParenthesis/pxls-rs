use http::{StatusCode, Response};
use serde::Serialize;
use url::Url;
use warp::{
	reject::Rejection,
	reply::{json, Reply, self},
	Filter,
};

use crate::{
	access::permissions::{with_permission, Permission},
	filters::header::{authorization, range::{self, Range}},
	filters::resource::database,
	filters::resource::board,
	filters::body::patch,
	database::Pool,
	objects::*,
};

pub mod auth;
pub mod core;
