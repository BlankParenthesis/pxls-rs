use std::collections::hash_map::Entry;
use std::collections::{HashMap, VecDeque};

#[derive(Debug, Clone, PartialEq, Eq)]
struct CachedActivityPixel {
	timestamp: u32,
	uid: i32,
}

pub struct ActivityCache {
	count_per_user: HashMap<i32, u32>, 
	latest_pixels: VecDeque<CachedActivityPixel>,
	idle_timeout: u32,
}

impl ActivityCache {
	pub fn new(idle_timeout: u32) -> Self {
		Self {
			count_per_user: HashMap::new(),
			latest_pixels: VecDeque::new(),
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
					match self.count_per_user.get_mut(&oldest.uid) {
						Some(v) if *v <= 1 => {
							self.count_per_user.remove(&oldest.uid);
						},
						Some(v) => {
							*v -= 1;
						},
						None => debug_assert!(false),
					}
					self.latest_pixels.pop_front();
				},
				_ => break,
			}
		}

		self.count_per_user.len()
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

		match self.count_per_user.entry(uid) {
			Entry::Occupied(mut occupied) => {
				*(occupied.get_mut()) += 1;
			},
			Entry::Vacant(vacant) => {
				vacant.insert(1);
			},
		}
		self.latest_pixels.push_back(CachedActivityPixel { timestamp, uid });
	}
}
