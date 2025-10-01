use std::collections::hash_map::Entry;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use std::collections::{HashMap, VecDeque};
use warp::http::header::{HeaderName, HeaderValue};

use crate::config::CONFIG;
use crate::database::UserSpecifier;


#[derive(Debug, Default, Clone, Copy)]
struct CacheEntry {
	activity: u32,
	density: u32,
	timestamp: u32,
	previous_stack: u32,
}

#[derive(Debug)]
pub struct CooldownCache {
	cache: HashMap<UserSpecifier, VecDeque<CacheEntry>>,
	max_pixels: u32,
	epoch: SystemTime,
}

impl CooldownCache {
	pub fn new(max_pixels: u32, epoch: SystemTime) -> Self {
		Self {
			cache: HashMap::new(),
			max_pixels,
			epoch,
		}
	}
	
	fn compute_cooldown(
		activity: u32,
		density: u32,
		stack: u32,
	) -> u32 {
		// TODO: proper cooldown
		CONFIG.cooldown * (stack + 1)
	}
	
	pub fn remove(&mut self, timestamp: u32, user: UserSpecifier) {
		if let Some(cooldown) = self.cache.get_mut(&user) {
			let mut popped_entries = vec![];
			
			// store all caches above the entry
			loop {
				match cooldown.back() {
					Some(CacheEntry { timestamp: entry_timestamp, .. })
					if *entry_timestamp > timestamp => {
						popped_entries.push(cooldown.pop_back().unwrap())
					},
					_ => break,
				}
			}
			
			// remove the entry (if it exists)
			cooldown.pop_back();
			
			// re-insert our popped entries, correctly calculating the new cooldown
			for CacheEntry { timestamp, activity, density, .. } in popped_entries {
				self.insert(timestamp, user, activity, density);
			}
		}
	}
	
	pub fn insert(
		&mut self,
		timestamp: u32,
		user: UserSpecifier,
		activity: u32,
		density: u32,
	) {
		let previous_stack = self.get(user, timestamp).pixels_available.saturating_sub(1);
		
		let entry = CacheEntry {
			activity,
			density,
			timestamp,
			previous_stack,
		};
		
		match self.cache.entry(user) {
			Entry::Occupied(mut occupied) => {
				let cache = occupied.get_mut();
				let min_timestamp = timestamp.saturating_sub(CONFIG.undo_deadline_seconds);
				
				loop {
					match cache.front() {
						Some(CacheEntry { timestamp, .. })
						if *timestamp < min_timestamp => {
							cache.pop_front();
						},
						_ => break,
					}
				}
				cache.push_back(entry);
				
			},
			Entry::Vacant(vacant) => {
				vacant.insert(VecDeque::from(vec![entry]));
			}
		}
	}
	
	pub fn get(&self, user: UserSpecifier, now: u32) -> CooldownInfo {
		let CacheEntry {
			activity,
			density,
			timestamp,
			previous_stack,
		} = self.cache.get(&user).and_then(|v| v.back()).copied()
			.unwrap_or_else(CacheEntry::default);

		let mut cooldowns = (previous_stack..self.max_pixels)
			.map(|stack| Self::compute_cooldown(activity, density, stack))
			.map(|cooldown| cooldown + timestamp)
			.skip_while(|time| *time <= now)
			.map(|time| self.epoch + Duration::from_secs(time as u64))
			.collect::<Vec<SystemTime>>();
		
		cooldowns.reverse();
	
		let pixels_available = self.max_pixels - cooldowns.len() as u32;
		
		CooldownInfo { cooldowns, pixels_available }
	}
}

#[derive(Clone, Debug)]
pub struct CooldownInfo {
	// a stack of cooldowns, such that pop() gets the next cooldown
	cooldowns: Vec<SystemTime>,
	pub pixels_available: u32,
}

impl CooldownInfo {
	pub fn into_headers(mut self) -> Vec<(HeaderName, HeaderValue)> {
		let mut headers = vec![(
			HeaderName::from_static("pxls-pixels-available"),
			self.pixels_available.into(),
		)];

		if let Some(next_available) = self.cooldowns.pop() {
			headers.push((
				HeaderName::from_static("pxls-next-available"),
				next_available.duration_since(UNIX_EPOCH).unwrap()
					.as_secs().into(),
			));
		}

		headers
	}
}

impl Iterator for CooldownInfo {
	type Item = (SystemTime, u32);

	fn next(&mut self) -> Option<Self::Item> {
		self.cooldowns.pop().map(|time| {
			let count = self.pixels_available;
			self.pixels_available += 1;
			(time, count)
		})
	}
}
