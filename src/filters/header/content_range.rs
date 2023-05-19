use std::convert::TryFrom;

use super::*;

pub struct ContentRange {
	pub unit: String,
	pub range: Option<(usize, usize)>,
	pub size: Option<usize>,
}

impl TryFrom<&str> for ContentRange {
	type Error = RangeParseError;

	fn try_from(header: &str) -> Result<Self, Self::Error> {
		let (unit, range_data) = header
			.split_once(' ')
			.ok_or(RangeParseError::MissingUnit)?;

		let unit = String::from(unit);

		let (range, size) = range_data
			.split_once('/')
			.ok_or(RangeParseError::MissingSize)?;

		let size = if size == "*" {
			None
		} else {
			Some(size.parse().map_err(RangeParseError::ValueParseError)?)
		};

		let range = if range == "*" {
			None
		} else {
			let (start, end) = range
				.split_once('-')
				.ok_or_else(|| RangeParseError::MissingHyphenMinus(range.to_owned()))?;

			let start = start
				.parse::<usize>()
				.map_err(RangeParseError::ValueParseError)?;
			let end = end
				.parse::<usize>()
				.map_err(RangeParseError::ValueParseError)?;

			Some((start, end))
		};

		Ok(Self { unit, size, range })
	}
}

pub fn content_range() -> impl Filter<Extract = (ContentRange,), Error = Rejection> + Copy {
	warp::header(header::CONTENT_RANGE.as_str())
		.and_then(|header: String| async move {
			ContentRange::try_from(header.as_str())
				.map_err(warp::reject::custom)
		})
}
