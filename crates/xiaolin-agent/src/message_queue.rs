use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

/// Max queued steering messages per agent run; excess low-priority messages are dropped.
const MAX_QUEUE_SIZE: usize = 10_000;

/// Priority levels for queued messages. Higher priority messages are drained first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Priority {
    /// Low-priority background notifications (e.g. sub-agent completion).
    Low = 0,
    /// Normal steering messages from other agents or the user.
    Normal = 1,
    /// High-priority system messages (e.g. budget limits, cancellation).
    High = 2,
}

/// A queued message waiting to be injected into an agent's context.
#[derive(Debug, Clone)]
pub struct QueuedMessage {
    pub priority: Priority,
    pub source: String,
    pub content: String,
    pub timestamp_ms: u64,
}

/// Thread-safe message queue for steering running agents.
///
/// Producers (SendMessageTool, gateway WS handler, completion hooks) push messages.
/// The agent loop drains messages at tool-round boundaries and injects them as
/// user-role steering messages before the next LLM call.
#[derive(Debug, Clone)]
pub struct MessageQueue {
    inner: Arc<Mutex<VecDeque<QueuedMessage>>>,
}

impl MessageQueue {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(VecDeque::new())),
        }
    }

    /// Push a message into the queue.
    ///
    /// When at capacity, drops the oldest low-priority message first; if the queue
    /// remains full and the incoming message is low priority, the new message is dropped.
    pub fn push(&self, priority: Priority, source: impl Into<String>, content: impl Into<String>) {
        let msg = QueuedMessage {
            priority,
            source: source.into(),
            content: content.into(),
            timestamp_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
        };
        let mut guard = self.inner.lock().expect("message queue lock poisoned");
        if guard.len() >= MAX_QUEUE_SIZE {
            let low_idx = guard
                .iter()
                .position(|m| m.priority == Priority::Low)
                .or_else(|| guard.iter().position(|m| m.priority == Priority::Normal));
            if let Some(idx) = low_idx {
                if let Some(dropped) = guard.remove(idx) {
                    tracing::warn!(
                        max = MAX_QUEUE_SIZE,
                        dropped_priority = ?dropped.priority,
                        "message queue at capacity; dropped lower-priority message"
                    );
                }
            } else if priority == Priority::Low {
                tracing::warn!(
                    max = MAX_QUEUE_SIZE,
                    "message queue at capacity; dropping incoming low-priority message"
                );
                return;
            } else {
                guard.pop_front();
                tracing::warn!(
                    max = MAX_QUEUE_SIZE,
                    "message queue at capacity; dropped oldest message"
                );
            }
        }
        guard.push_back(msg);
    }

    /// Drain all messages up to (and including) `max_priority`.
    /// Returns messages sorted by priority (highest first), then by insertion order.
    pub fn drain(&self, max_priority: Priority) -> Vec<QueuedMessage> {
        let mut guard = self.inner.lock().expect("message queue lock poisoned");
        let mut drained = Vec::new();
        let mut remaining = VecDeque::new();

        while let Some(msg) = guard.pop_front() {
            if msg.priority <= max_priority {
                drained.push(msg);
            } else {
                remaining.push_back(msg);
            }
        }

        *guard = remaining;
        drained.sort_by(|a, b| b.priority.cmp(&a.priority));
        drained
    }

    /// Drain all pending messages regardless of priority.
    pub fn drain_all(&self) -> Vec<QueuedMessage> {
        self.drain(Priority::High)
    }

    /// Check if the queue has any pending messages.
    pub fn is_empty(&self) -> bool {
        self.inner
            .lock()
            .expect("message queue lock poisoned")
            .is_empty()
    }

    /// Number of pending messages.
    pub fn len(&self) -> usize {
        self.inner
            .lock()
            .expect("message queue lock poisoned")
            .len()
    }
}

impl Default for MessageQueue {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_and_drain_respects_priority_ordering() {
        let q = MessageQueue::new();
        q.push(Priority::Low, "bg", "low msg");
        q.push(Priority::High, "system", "high msg");
        q.push(Priority::Normal, "user", "normal msg");

        let msgs = q.drain_all();
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[0].priority, Priority::High);
        assert_eq!(msgs[1].priority, Priority::Normal);
        assert_eq!(msgs[2].priority, Priority::Low);
    }

    #[test]
    fn drain_with_max_priority_filters_correctly() {
        let q = MessageQueue::new();
        q.push(Priority::Low, "a", "lo");
        q.push(Priority::High, "b", "hi");
        q.push(Priority::Normal, "c", "norm");

        let msgs = q.drain(Priority::Normal);
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].priority, Priority::Normal);
        assert_eq!(msgs[1].priority, Priority::Low);

        // High priority remains
        assert_eq!(q.len(), 1);
        let remaining = q.drain_all();
        assert_eq!(remaining[0].priority, Priority::High);
    }

    #[test]
    fn drain_empties_queue() {
        let q = MessageQueue::new();
        q.push(Priority::Normal, "src", "msg");
        assert!(!q.is_empty());
        let _ = q.drain_all();
        assert!(q.is_empty());
    }
}
