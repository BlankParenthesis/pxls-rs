use std::time::{SystemTime, UNIX_EPOCH};
use http::header::{HeaderName, HeaderValue};

#[derive(Clone, Debug)]
pub struct CooldownInfo {
	cooldowns: Vec<SystemTime>,
	pub pixels_available: usize,
}

impl CooldownInfo {
	pub fn new(
		cooldowns: Vec<SystemTime>,
		current_timestamp: SystemTime,
	) -> Self {
		let pixels_available = cooldowns
			.iter()
			.enumerate()
			.take_while(|(_, cooldown)| **cooldown <= current_timestamp)
			.last()
			.map(|(i, _)| i + 1)
			.unwrap_or(0);

		Self {
			cooldowns,
			pixels_available,
		}
	}

	pub fn into_headers(self) -> Vec<(HeaderName, HeaderValue)> {
		let mut headers = vec![(
			HeaderName::from_static("pxls-pixels-available"),
			self.pixels_available.into(),
		)];

		if let Some(next_available) = self
			.cooldowns
			.get(self.pixels_available)
		{
			headers.push((
				HeaderName::from_static("pxls-next-available"),
				(*next_available)
					.duration_since(UNIX_EPOCH)
					.unwrap()
					.as_secs()
					.into(),
			));
		}

		headers
	}

	pub fn cooldown(&self) -> Option<SystemTime> {
		self.cooldowns
			.get(self.pixels_available)
			.map(SystemTime::clone)
	}
}

impl Iterator for CooldownInfo {
	type Item = SystemTime;

	fn next(&mut self) -> Option<Self::Item> {
		let time = self.cooldown();
		if time.is_some() {
			self.pixels_available += 1;
		}
		time
	}
}