use std::time::{Duration, Instant};

use dashmap::DashMap;

const MAX_ENTRIES: usize = 10_000;
const TTL: Duration = Duration::from_secs(300);

/// Deduplicates WeChat messages by message_id within a time window.
pub struct MessageDedup {
    seen: DashMap<String, Instant>,
}

impl MessageDedup {
    pub fn new() -> Self {
        Self {
            seen: DashMap::new(),
        }
    }

    /// Returns true if this message_id has NOT been seen (i.e., is new).
    pub fn accept(&self, message_id: &str) -> bool {
        self.evict_expired();
        if self.seen.contains_key(message_id) {
            return false;
        }
        if self.seen.len() >= MAX_ENTRIES {
            self.evict_oldest();
        }
        self.seen.insert(message_id.to_string(), Instant::now());
        true
    }

    fn evict_expired(&self) {
        self.seen.retain(|_, ts| ts.elapsed() < TTL);
    }

    fn evict_oldest(&self) {
        if let Some(oldest_key) = self
            .seen
            .iter()
            .min_by_key(|e| *e.value())
            .map(|e| e.key().clone())
        {
            self.seen.remove(&oldest_key);
        }
    }
}

impl Default for MessageDedup {
    fn default() -> Self {
        Self::new()
    }
}
