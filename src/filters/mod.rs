pub mod header;
pub mod resource;
pub mod body;

use std::num::ParseIntError;
use std::convert::Infallible;

use http::{StatusCode, Response};
use warp::{reject::{Reject, Rejection}, Reply, reply::{self}, Filter};