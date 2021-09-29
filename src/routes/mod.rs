use actix_web::*;
use serde::Serialize;
use serde_qs::actix::QsQuery;
use ::http::StatusCode;
use url::Url;

use crate::objects::*;
use crate::access::permissions::Permission;
use crate::database::Pool;

pub mod core;
pub mod auth;