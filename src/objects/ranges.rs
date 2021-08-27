use std::num::ParseIntError;
use std::ops::Range;
use std::fmt::{self, Display, Formatter};
use actix_web::web::{Bytes, BytesMut};

#[derive(Debug)]
pub enum RangeParseError {
	MissingUnit,
	MissingHyphenMinus(String),
	ValueParseError(ParseIntError),
	RangeEmpty,
}

impl Display for RangeParseError {
	fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
		write!(f, "invalid range format: {}", match self {
			Self::MissingUnit => "missing unit",
			Self::MissingHyphenMinus(range) => "missing hyphen-minus",
			Self::ValueParseError(err) => "value parse error",
			Self::RangeEmpty => "empty range",
		})
	}
}

pub struct Ranges<'l, T> {
	of: &'l T,
	unit: String, 
	len: usize,
	ranges: Vec<Range<usize>>,
}

impl<'l, T> Ranges<'l, T>
where T: AsRef<[u8]> {
	pub fn iter<'il>(&'il self) -> RangesIterator<'il, 'l, T> {
		RangesIterator {
			current: 0,
			ranges: self,
		}
	}

	pub fn len(&self) -> usize {
		self.len
	}

	pub fn parse(header: &str, of: &'l T) -> Result<Self, RangeParseError> {
		let split_header = header.split_once('=');
		let (unit, range_data) = split_header.ok_or(RangeParseError::MissingUnit)?;
		let unit = String::from(unit);

		let ranges: Vec<Range<_>> = range_data.split(',')
			.map(|range| {
				let range = range.split_once('-').ok_or(
					RangeParseError::MissingHyphenMinus(String::from(range))
				)?;
				match range {
					("", "") => Err(RangeParseError::RangeEmpty),
					("", since_end) => {
						let since_end: usize = since_end.parse()
							.map_err(|err| RangeParseError::ValueParseError(err))?;
						let end = of.as_ref().len();
						let start = end - since_end;
						Ok(start..end)
					},
					(start, "") => {
						let start = start.parse()
							.map_err(|err| RangeParseError::ValueParseError(err))?;
						let end = of.as_ref().len();
						Ok(start..end)
					},
					(start, end) => {
						let start = start.parse()
							.map_err(|err| RangeParseError::ValueParseError(err))?;
						let end = end.parse()
							.map_err(|err| RangeParseError::ValueParseError(err))?;
						Ok(start..end)
					},
				}
			})
			.collect::<Result<_, _>>()?;

		let len = ranges.iter().fold(0, |count, range| count + range.len());

		Ok(Self { of, unit, ranges, len })
	}
}

impl<'l, T> From<&Ranges<'l, T>> for Bytes
where T: AsRef<[u8]> {
	fn from(ranges: &Ranges<'l, T>) -> Self {
		ranges.iter()
			.fold(BytesMut::with_capacity(ranges.len()), |mut bytes, slice| {
				bytes.extend(slice);
				bytes
			})
			.freeze()
	}
}

pub struct RangesIterator<'il, 'l, T> {
	current: usize,
	ranges: &'il Ranges<'l, T>,
}

impl<'il, 'l, T> Iterator for RangesIterator<'il, 'l, T>
where T: AsRef<[u8]> {
	type Item = &'l [u8];

	fn next(&mut self) -> Option<Self::Item> {
		let index = self.current;
		self.current += 1;
		self.ranges.ranges.get(index)
			.map(|range| {
				&self.ranges.of.as_ref()[range.clone()]
			})
	}
}
