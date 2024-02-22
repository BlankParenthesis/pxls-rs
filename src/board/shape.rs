use std::ops::Range;

use serde::{Serialize, Deserialize};

#[derive(Debug, Clone)]
pub struct Shape {
	sector_size: usize,
	sector_count: usize,
	total_size: usize,
	dimensionality: usize,
	dimensions: Vec<usize>,
	shape: Vec<Vec<usize>>,
}

impl Shape {
	pub fn new(shape: Vec<Vec<usize>>) -> Self {
		let sector_size = shape.iter()
				.last()
				.map(|last| last.iter().product())
				.unwrap_or(0);
	
		let sector_count = shape.iter()
				.rev()
				.skip(1)
				.map(|items| items.iter().product::<usize>())
				.product();
	
		let total_size = sector_count * sector_size;

		let dimensionality = shape.iter().map(Vec::len).max().unwrap_or(0);
	
		let mut dimensions = vec![1; dimensionality];

		for level in shape.iter() {
			for (i, axis) in dimensions.iter_mut().enumerate() {
				*axis *= level.get(i).copied().unwrap_or(1);
			}
		}

		Self {
			sector_size,
			sector_count,
			total_size,
			dimensionality,
			dimensions,
			shape,
		}
	}

	pub fn sector_size(&self) -> usize { self.sector_size }
	pub fn sector_count(&self) -> usize { self.sector_count }
	pub fn total_size(&self) -> usize { self.total_size }
	pub fn dimensions(&self) -> &[usize] { &self.dimensions }
	
	pub fn depth(&self) -> usize { self.shape.len() }
	pub fn dimensionality(&self) -> usize { self.dimensionality }
	
	pub fn transform(
		from: &Self,
		to: &Self,
		position: usize,
	) -> usize {
		todo!("implement shape-to-shape transforming")
	}

	pub fn to_local(&self, position: usize) -> Option<(usize, usize)> {
		if self.contains(&position) {
			let size = self.sector_size();
			Some((position / size, position % size))
		} else {
			None
		}
	}

	pub fn contains(&self, position: &usize) -> bool {
		(0..self.total_size()).contains(position)
	}

	pub fn sectors_within(&self, range: Range<usize>) -> Range<usize> {
		let sector_size = self.sector_size();
		let start = range.start / sector_size;
		let end = range.end / sector_size;
		start..end.min(self.sector_count())
	}
}

impl Serialize for Shape {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: serde::Serializer,
	{
		self.shape.serialize(serializer)
	}
}

impl<'de> Deserialize<'de> for Shape {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where
		D: serde::Deserializer<'de>,
	{
		Vec::<Vec<usize>>::deserialize(deserializer).map(Self::new)
	}
}