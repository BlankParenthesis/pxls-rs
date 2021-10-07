use std::ops::Range;

pub trait Shape {
	fn sector_size(&self) -> usize;
	fn sector_count(&self) -> usize;
	fn total_size(&self) -> usize;
	fn dimensions(&self) -> usize;
	fn transform(from: &Self, to: &Self, position: usize) -> usize;
	fn to_local(&self, position: usize) -> Option<(usize, usize)>;

	fn contains(&self, position: &usize) -> bool {
		(0..self.total_size()).contains(position)
	}

	fn sectors_within(&self, range: Range<usize>) -> Range<usize> {
		let sector_size = self.sector_size();
		let start = range.start / sector_size;
		let end = range.end / sector_size;
		start..end.min(self.sector_count())
	}
}

pub type VecShape = Vec<Vec<usize>>;

// TODO: StructShape (or something) which stores these values on new
// rather than recomputing them.
impl Shape for VecShape {
	fn sector_size(&self) -> usize {
		self.iter()
			.last()
			.map(|last| last
				.iter()
				.map(|item| *item as usize)
				.product())
			.unwrap_or(0)
	}

	fn sector_count(&self) -> usize {
		self.iter()
			.rev()
			.skip(1)
			.map(|items| items
				.iter()
				.map(|item| *item as usize)
				.product::<usize>())
			.product()
	}

	fn total_size(&self) -> usize {
		self.sector_count() * self.sector_size()
	}


	fn dimensions(&self) -> usize {
		self.len()
	}

	fn transform(from: &Self, to: &Self, position: usize) -> usize {
		todo!("implement shape-to-shape transforming")
	}

	fn to_local(&self, position: usize) -> Option<(usize, usize)> {
		if self.contains(&position) {
			let size = self.sector_size();
			Some((
				position / size,
				position % size, 
			))
		} else {
			None
		}
	}
}