pub mod body;
pub mod header;
pub mod resource;

use std::{convert::Infallible, num::ParseIntError};

use http::{Response, StatusCode};
use warp::{
	reject::{Reject, Rejection},
	reply::{self},
	Filter, Reply,
};
