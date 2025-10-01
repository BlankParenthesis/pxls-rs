use std::fmt;
use std::marker::PhantomData;

use reqwest::StatusCode;
use sea_query::SimpleExpr;
use warp::filters::path::FullPath;
use warp::reject::{self, Reject, Rejection};
use warp::http::Uri;
use warp::Filter;

#[derive(Debug)]
pub enum SpecfierParseError {
	PathNotAbsolute,
	Expected(&'static PathPart),
	UnexpectedEnd,
	ExpectedEnd,
	ParseFailed(std::num::ParseIntError),
}

impl From<std::num::ParseIntError> for SpecfierParseError {
	fn from(value: std::num::ParseIntError) -> Self {
		Self::ParseFailed(value)
	}
}

impl fmt::Display for SpecfierParseError {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		// TODO: nicer errors
		write!(f, "{self:?}")
	}
}

impl From<&SpecfierParseError> for StatusCode {
	fn from(value: &SpecfierParseError) -> Self {
		match value {
			SpecfierParseError::ParseFailed(_) => StatusCode::BAD_REQUEST,
			_ => StatusCode::NOT_FOUND,
		}
	}
}

impl Reject for SpecfierParseError {}

macro_rules! path_part {
	( $part:literal ) => {
		PathPart::Path($part)
	};
	( $part:expr ) => {
		PathPart::Value
	};
}

pub(crate) use path_part;

macro_rules! specifier_path {
	( $( $x:expr ),* ) => {
		{
			use crate::database::specifier::path_part;
			&[ $( path_part!($x) ),* ]
		}
    };
}

pub(crate) use specifier_path;

#[derive(Debug)]
pub enum PathPart {
	Path(&'static str),
	Value,
}

#[derive(Debug)]
pub enum Id {
	I32(i32),
	U64(u64),
}

impl fmt::Display for Id {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Id::I32(v) => write!(f, "{v}"),
			Id::U64(v) => write!(f, "{v}"),
		}
	}
}

pub trait Specifier: Sized {
	fn filter(&self) -> SimpleExpr;
	
	fn parts() -> &'static [PathPart];
	fn from_ids(ids: &[&str]) -> Result<Self, SpecfierParseError>;
	fn to_ids(&self) -> Box<[Id]>;
	
	fn from_uri(uri: Uri) -> Result<Self, SpecfierParseError> {
		let mut segments = uri.path().split('/');
		
		if !matches!(segments.next(), Some("")) {
			return Err(SpecfierParseError::PathNotAbsolute)
		}
		
		let mut values = vec![];
		
		for part in Self::parts() {
			let next = segments.next().ok_or(SpecfierParseError::UnexpectedEnd)?;
			match part {
				PathPart::Path(path) => {
					if next != *path {
						return Err(SpecfierParseError::Expected(part))
					}
				},
				PathPart::Value => {
					values.push(next)
				}
			}
		}
		
		if segments.next().is_some() {
			return Err(SpecfierParseError::ExpectedEnd);
		}
		
		Self::from_ids(&values)
	}
	
	fn to_uri(&self) -> Uri {
		let ids = self.to_ids();
		let mut ids = ids.iter();
		let mut uri = String::new();
		for part in Self::parts() {
			uri.push('/');
			match part {
				PathPart::Path(path) => uri.push_str(path),
				PathPart::Value => {
					let id = ids.next().expect("Specifier demands more values than it provides");
					uri.push_str(&id.to_string());
				}
			}
		}
		uri.parse().unwrap()
	}
	
	fn path() -> impl Filter<
		Extract = (Self,),
		Error = Rejection
	> + Clone {
		warp::path::full()
			.and_then(|path: FullPath| async move {
				let uri = path.as_str().parse()
					.map_err(|_| warp::reject::not_found())?;
				Self::from_uri(uri)
					.map_err(|e| match e {
						SpecfierParseError::ParseFailed(_) => reject::custom(e),
						_ => reject::not_found()
					})
			})
	}
}

pub struct SpecifierParser<S> {
	expecting: &'static str,
	_specifier: PhantomData<S>,
}

impl<O> SpecifierParser<O> {
	pub fn new(expecting: &'static str) -> Self {
		Self { expecting, _specifier: PhantomData }
	}
}

impl<'de, S: Specifier> serde::de::Visitor<'de> for SpecifierParser<S> {
	type Value = S;
	
	fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
		write!(f, "{}", self.expecting)
	}

	fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
	where E: serde::de::Error, {
		let uri = v.parse::<Uri>().map_err(E::custom)?;
		S::from_uri(uri).map_err(E::custom)
	}
}
