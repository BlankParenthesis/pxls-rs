use async_trait::async_trait;

pub mod buffer;
pub mod cache;
pub mod access;

pub use buffer::{Sector, SectorBuffer};
pub use cache::SectorCache;
pub use access::SectorAccessor;

#[async_trait]
pub trait AsyncRead {
	type Error;

	async fn read(
		&mut self,
		output: &mut [u8],
	) -> std::result::Result<usize, Self::Error>;
}

#[async_trait]
pub trait AsyncWrite {
	type Error;

	async fn write(
		&mut self,
		input: &[u8],
	) -> std::result::Result<usize, Self::Error>;

	async fn flush(&mut self) -> std::result::Result<(), Self::Error>;
}

pub trait Len {
	fn len(&self) -> usize;

	fn is_empty(&self) -> bool {
		self.len() == 0
	}
}