use std::collections::{HashSet, VecDeque};

#[derive(Debug, Clone, PartialEq, Eq)]
struct CachedActivityPixel {
	timestamp: u32,
	uid: i32,
}

pub struct ActivityCache {
	latest_pixels: VecDeque<CachedActivityPixel>,
	cached_activity: Option<usize>,
	idle_timeout: u32,
}

impl ActivityCache {
	pub fn new(idle_timeout: u32) -> Self {
		Self {
			latest_pixels: VecDeque::new(),
			cached_activity: None,
			idle_timeout,
		}
	}

	pub fn count(&mut self, now: u32) -> usize {
		debug_assert!(
			self.latest_pixels.back()
				.map(|b| b.timestamp <= now)
				.unwrap_or(true)
		);

		let idle_start = now.saturating_sub(self.idle_timeout);

		loop {
			match self.latest_pixels.front() {
				Some(oldest) if oldest.timestamp < idle_start => {
					self.latest_pixels.pop_front();
					self.cached_activity = None;
				},
				_ => break,
			}
		}

		*self.cached_activity.get_or_insert_with(|| {
			let mut users = HashSet::new();
			for CachedActivityPixel { uid, .. } in self.latest_pixels.iter() {
				users.insert(uid);
			}
			users.len()
		})
	}

	pub fn remove(&mut self, timestamp: u32, uid: i32) {
		let pixel = CachedActivityPixel { timestamp, uid };
		let position = self.latest_pixels.iter().position(|p| *p == pixel);
		if let Some(index) = position {
			self.latest_pixels.remove(index);
		}
	}

	pub fn insert(&mut self, timestamp: u32, uid: i32) {
		debug_assert!(
			self.latest_pixels.back()
				.map(|b| b.timestamp <= timestamp)
				.unwrap_or(true)
		);

		self.cached_activity = None;
		self.latest_pixels.push_back(CachedActivityPixel { timestamp, uid });
	}
}
