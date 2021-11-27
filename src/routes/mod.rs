use ::http::StatusCode;
use actix_web::*;
use serde::Serialize;
use serde_qs::actix::QsQuery;
use url::Url;

use crate::{access::permissions::Permission, database::Pool, objects::*};

pub mod auth;
pub mod core;
