use std::collections::HashMap;
use std::time::{Duration, Instant};

const MAX_ENTRIES: usize = 10_000;

/// Deduplicates Feishu messages by message_id within a time window.
pub struct MessageDedup {
    seen: HashMap<String, Instant>,
    ttl: Duration,
}

impl MessageDedup {
    pub fn new(ttl: Duration) -> Self {
        Self {
            seen: HashMap::new(),
            ttl,
        }
    }

    /// Returns true if this message_id has NOT been seen (i.e., is new).
    pub fn check(&mut self, message_id: &str) -> bool {
        self.evict();
        if self.seen.contains_key(message_id) {
            return false;
        }
        if self.seen.len() >= MAX_ENTRIES {
            self.evict_oldest();
        }
        self.seen.insert(message_id.to_string(), Instant::now());
        true
    }

    /// Check if a message has expired (older than ttl).
    pub fn is_expired(created: Instant, ttl: Duration) -> bool {
        created.elapsed() > ttl
    }

    pub fn size(&self) -> usize {
        self.seen.len()
    }

    fn evict(&mut self) {
        let ttl = self.ttl;
        self.seen.retain(|_, ts| ts.elapsed() < ttl);
    }

    fn evict_oldest(&mut self) {
        if let Some(oldest_key) = self
            .seen
            .iter()
            .min_by_key(|(_, ts)| *ts)
            .map(|(k, _)| k.clone())
        {
            self.seen.remove(&oldest_key);
            tracing::warn!(
                max = MAX_ENTRIES,
                removed = %oldest_key,
                remaining = self.seen.len(),
                "MessageDedup at capacity; evicted oldest entry"
            );
        }
    }

    pub fn dispose(&mut self) {
        self.seen.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dedup_new_message() {
        let mut dedup = MessageDedup::new(Duration::from_secs(60));
        assert!(dedup.check("om_1"));
        assert!(!dedup.check("om_1"));
        assert!(dedup.check("om_2"));
    }

    #[test]
    fn dedup_size() {
        let mut dedup = MessageDedup::new(Duration::from_secs(60));
        dedup.check("om_1");
        dedup.check("om_2");
        assert_eq!(dedup.size(), 2);
    }

    #[test]
    fn dispose_clears() {
        let mut dedup = MessageDedup::new(Duration::from_secs(60));
        dedup.check("om_1");
        dedup.dispose();
        assert_eq!(dedup.size(), 0);
        assert!(dedup.check("om_1"));
    }
}
