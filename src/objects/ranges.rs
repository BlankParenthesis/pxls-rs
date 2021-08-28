use std::num::ParseIntError;
use std::ops::{Range, RangeFrom};
use std::fmt::{self, Display, Formatter};

#[derive(Debug)]
pub enum RangeParseError {
	MissingUnit,
	MissingHyphenMinus(String),
	ValueParseError(ParseIntError),
	RangeEmpty,
	NoRange,
	Backwards,
}

impl Display for RangeParseError {
	fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
		write!(f, "invalid range format: {}", match self {
			Self::MissingUnit => "missing unit",
			Self::MissingHyphenMinus(range) => "missing hyphen-minus",
			Self::ValueParseError(err) => "value parse error",
			Self::RangeEmpty => "empty range",
			Self::NoRange => "no ranges",
			Self::Backwards => "range ended before beginning",
		})
	}
}

pub enum HttpRange {
	FromStartToEnd(Range<usize>),
	FromStartToLast(RangeFrom<usize>),
	FromEndToLast(usize),
}

pub enum Ranges {
	Multi {
		unit: String, 
		ranges: Vec<HttpRange>,
	},
	Single {
		unit: String, 
		range: HttpRange,
	},
}

impl Ranges {
	pub fn parse(header: &str) -> Result<Self, RangeParseError> {
		let split_header = header.split_once('=');
		let (unit, range_data) = split_header.ok_or(RangeParseError::MissingUnit)?;
		let unit = String::from(unit);

		let mut ranges: Vec<HttpRange> = range_data.split(',')
			.map(|range| {
				let tuple = range.split_once('-').ok_or(
					RangeParseError::MissingHyphenMinus(String::from(range))
				)?;
				let http_range = match tuple {
					("", "") => Err(RangeParseError::RangeEmpty),
					("", since_end) => since_end.parse()
						.map(HttpRange::FromEndToLast)
						.map_err(RangeParseError::ValueParseError),
					(start, "") => start.parse()
						.map(|start| HttpRange::FromStartToLast(start..))
						.map_err(RangeParseError::ValueParseError),
					(start, end) => start.parse()
						.and_then(|start| end.parse()
						.map(|end| HttpRange::FromStartToEnd(start..end)))
						.map_err(RangeParseError::ValueParseError),
				}?;
				if let HttpRange::FromStartToEnd(range) = &http_range {
					if range.end < range.start {
						Err(RangeParseError::Backwards)
					} else {
						Ok(http_range)
					}
				} else {
					Ok(http_range)
				}
			})
			.collect::<Result<_, _>>()?;

		match ranges.len() {
			0 => Err(RangeParseError::NoRange),
			1 => Ok(Self::Single { unit, range: ranges.swap_remove(0) }),
			len => Ok(Self::Multi { unit, ranges }),
		}
	}
}
