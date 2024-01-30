use std::ops::Range;

use serde::{Serialize, Deserialize};

pub trait Shape {
	fn sector_size(&self) -> usize;
	fn sector_count(&self) -> usize;
	fn total_size(&self) -> usize;
	fn dimensions(&self) -> usize;
	fn transform(
		from: &Self,
		to: &Self,
		position: usize,
	) -> usize;
	fn to_local(
		&self,
		position: usize,
	) -> Option<(usize, usize)>;

	fn contains(
		&self,
		position: &usize,
	) -> bool {
		(0..self.total_size()).contains(position)
	}

	fn sectors_within(
		&self,
		range: Range<usize>,
	) -> Range<usize> {
		let sector_size = self.sector_size();
		let start = range.start / sector_size;
		let end = range.end / sector_size;
		start..end.min(self.sector_count())
	}
}

pub type VecShape = Vec<Vec<usize>>;

impl Shape for VecShape {
	fn sector_size(&self) -> usize {
		self.iter()
			.last()
			.map(|last| last.iter().product())
			.unwrap_or(0)
	}

	fn sector_count(&self) -> usize {
		self.iter()
			.rev()
			.skip(1)
			.map(|items| items.iter().product::<usize>())
			.product()
	}

	fn total_size(&self) -> usize {
		self.sector_count() * self.sector_size()
	}

	fn dimensions(&self) -> usize {
		self.len()
	}

	fn transform(
		from: &Self,
		to: &Self,
		position: usize,
	) -> usize {
		todo!("implement shape-to-shape transforming")
	}

	fn to_local(
		&self,
		position: usize,
	) -> Option<(usize, usize)> {
		if self.contains(&position) {
			let size = self.sector_size();
			Some((position / size, position % size))
		} else {
			None
		}
	}
}

#[derive(Debug, Clone)]
pub struct CachedVecShape {
	sector_size: usize,
	sector_count: usize,
	total_size: usize,
	dimensions: usize,
	shape: VecShape,
}

impl Shape for CachedVecShape {
	fn sector_size(&self) -> usize { self.sector_size }
	fn sector_count(&self) -> usize { self.sector_count }
	fn total_size(&self) -> usize { self.total_size }
	fn dimensions(&self) -> usize { self.dimensions }

	fn transform(from: &Self, to: &Self, position: usize,) -> usize {
		VecShape::transform(&from.shape, &to.shape, position)
	}

	fn to_local(&self, position: usize) -> Option<(usize, usize)> {
		self.shape.to_local(position)
	}
}

impl From<VecShape> for CachedVecShape {
	fn from(value: VecShape) -> Self {
		Self {
			sector_size: value.sector_size(),
			sector_count: value.sector_count(),
			total_size: value.total_size(),
			dimensions: value.dimensions(),
			shape: value,
		}
	}
}

impl Serialize for CachedVecShape {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: serde::Serializer,
	{
		self.shape.serialize(serializer)
	}
}

impl<'de> Deserialize<'de> for CachedVecShape {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
	{
        VecShape::deserialize(deserializer).map(Self::from)
    }
}