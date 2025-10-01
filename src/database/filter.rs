use std::fmt;
use std::str::FromStr;

use serde::Deserialize;
use serde::de::Visitor;

#[derive(Debug)]
pub struct FilterRange<T> {
	pub start: Option<T>,
	pub end: Option<T>,
}

impl<T: Ord + Copy> FilterRange<T> {
	pub fn contains(&self, value: T) -> bool {
		let after_start = if let Some(start) = self.start {
			value >= start
		} else {
			true
		};
		let before_end = if let Some(end) = self.end {
			value <= end
		} else {
			true
		};

		after_start && before_end
	}

	pub fn is_open(&self) -> bool {
		self.start.is_none() && self.end.is_none()
	}
}

impl<T: fmt::Display + Eq> fmt::Display for FilterRange<T> {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			FilterRange { start: Some(start), end: Some(end) } => {
				if start == end {
					write!(f, "{}", end)
				} else {
					write!(f, "{}..{}", start, end)
				}
			},
			FilterRange { start: Some(start), end: None } => {
				write!(f, "{}..", start)
			},
			FilterRange { start: None, end: Some(end) } => {
				write!(f, "..{}", end)
			},
			FilterRange { start: None, end: None } => {
				write!(f, "")
			},
		}
	}
}

impl<T> Default for FilterRange<T> {
	fn default() -> Self {
		Self { start: None, end: None }
	}
}

struct FilterRangeVisitor<T> {
	_marker: std::marker::PhantomData<T>
}

impl<'de, T, ET> Visitor<'de> for FilterRangeVisitor<T> 
where T: FromStr<Err = ET> + Copy, ET: fmt::Display {
	type Value = FilterRange<T>;

	fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
		write!(f, "A number or range")
	}

	fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
	where E: serde::de::Error, {
		if v.is_empty() {
			return Ok(Self::Value::default());
		}

		match v.split_once("..") {
			Some((start, end)) => {
				let start = if start.is_empty() {
					None
				} else {
					Some(start.parse().map_err(E::custom)?)
				};

				let end = if end.is_empty() {
					None
				} else {
					Some(end.parse().map_err(E::custom)?)
				};
		
				Ok(Self::Value { start, end })
			},
			None => {
				let number = Some(v.parse().map_err(E::custom)?);

				Ok(Self::Value { start: number, end: number })
			},
		}
	}
}

impl<'de, T, ET> Deserialize<'de> for FilterRange<T>
where
	T: FromStr<Err = ET> + Deserialize<'de> + Copy,
	ET: fmt::Display,
{
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where D: serde::Deserializer<'de> {
		let visitor = FilterRangeVisitor { _marker: Default::default() };
		deserializer.deserialize_str(visitor)
	}
}
