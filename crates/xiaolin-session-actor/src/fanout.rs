use std::sync::Arc;

use parking_lot::Mutex;
use tokio::sync::mpsc;

use crate::submission::SessionEvent;

/// Thread-safe shared reference to an `EventFanout`, allowing external callers
/// (like `SessionHandle`) to subscribe without going through the actor loop.
pub type SharedFanout = Arc<Mutex<EventFanout>>;

/// Strategy for handling backpressure when a subscriber's channel is full.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackpressurePolicy {
    /// Drop the event silently (suitable for best-effort WS clients).
    Drop,
    /// Block until the subscriber consumes (suitable for EventLog persistence).
    Block,
}

/// Multi-subscriber event fan-out.
///
/// Codex's EQ has a single consumer. XiaoLin needs to fan out events to
/// multiple subscribers (WS connections, SSE streams, EventLog, Feishu
/// channels) with per-subscriber backpressure policies.
pub struct EventFanout {
    subscribers: Vec<Subscriber>,
}

struct Subscriber {
    id: u64,
    tx: mpsc::Sender<SessionEvent>,
    policy: BackpressurePolicy,
}

impl EventFanout {
    pub fn new() -> Self {
        Self {
            subscribers: Vec::new(),
        }
    }

    /// Add a subscriber. Returns the subscriber ID for later removal and the
    /// receiving end.
    pub fn subscribe(
        &mut self,
        buffer: usize,
        policy: BackpressurePolicy,
    ) -> (u64, mpsc::Receiver<SessionEvent>) {
        let id = self.next_id();
        let (tx, rx) = mpsc::channel(buffer);
        self.subscribers.push(Subscriber { id, tx, policy });
        (id, rx)
    }

    /// Remove a subscriber by ID.
    pub fn unsubscribe(&mut self, id: u64) {
        self.subscribers.retain(|s| s.id != id);
    }

    /// Send an event to all subscribers.
    pub async fn send(&self, event: &SessionEvent) {
        for sub in &self.subscribers {
            if sub.tx.is_closed() {
                continue;
            }
            match sub.policy {
                BackpressurePolicy::Drop => {
                    let _ = sub.tx.try_send(event.clone());
                }
                BackpressurePolicy::Block => {
                    let _ = sub.tx.send(event.clone()).await;
                }
            }
        }
    }

    /// Remove closed subscribers.
    pub fn gc(&mut self) {
        self.subscribers.retain(|s| !s.tx.is_closed());
    }

    pub fn subscriber_count(&self) -> usize {
        self.subscribers.len()
    }

    /// Get cloned senders for all subscribers (used by relay tasks).
    pub(crate) fn subscriber_senders(&self) -> Vec<mpsc::Sender<SessionEvent>> {
        self.subscribers.iter().map(|s| s.tx.clone()).collect()
    }

    fn next_id(&self) -> u64 {
        self.subscribers
            .iter()
            .map(|s| s.id)
            .max()
            .unwrap_or(0)
            + 1
    }
}

impl Default for EventFanout {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xiaolin_protocol::id::{SessionId, SubmissionId, TurnId};
    use xiaolin_protocol::AgentEvent;

    fn test_event() -> SessionEvent {
        SessionEvent {
            id: SubmissionId::new("sub-1"),
            session_id: SessionId::new("sess-1"),
            msg: AgentEvent::TurnStart {
                turn_id: TurnId::new("t-1"),
                session_id: Some("sess-1".into()),
            },
        }
    }

    #[tokio::test]
    async fn single_subscriber_receives_event() {
        let mut fanout = EventFanout::new();
        let (_, mut rx) = fanout.subscribe(16, BackpressurePolicy::Drop);

        fanout.send(&test_event()).await;

        let event = rx.recv().await.unwrap();
        assert_eq!(event.session_id, "sess-1");
    }

    #[tokio::test]
    async fn multiple_subscribers_all_receive() {
        let mut fanout = EventFanout::new();
        let (_, mut rx1) = fanout.subscribe(16, BackpressurePolicy::Drop);
        let (_, mut rx2) = fanout.subscribe(16, BackpressurePolicy::Block);

        fanout.send(&test_event()).await;

        assert!(rx1.recv().await.is_some());
        assert!(rx2.recv().await.is_some());
    }

    #[tokio::test]
    async fn unsubscribe_removes_subscriber() {
        let mut fanout = EventFanout::new();
        let (id, _rx) = fanout.subscribe(16, BackpressurePolicy::Drop);
        assert_eq!(fanout.subscriber_count(), 1);

        fanout.unsubscribe(id);
        assert_eq!(fanout.subscriber_count(), 0);
    }

    #[tokio::test]
    async fn drop_policy_does_not_block() {
        let mut fanout = EventFanout::new();
        let (_id, _rx) = fanout.subscribe(1, BackpressurePolicy::Drop);

        // Fill the buffer
        fanout.send(&test_event()).await;
        // This should not block even though buffer is full
        fanout.send(&test_event()).await;
    }

    #[tokio::test]
    async fn gc_removes_closed_subscribers() {
        let mut fanout = EventFanout::new();
        let (_, rx) = fanout.subscribe(16, BackpressurePolicy::Drop);
        assert_eq!(fanout.subscriber_count(), 1);

        drop(rx);
        fanout.gc();
        assert_eq!(fanout.subscriber_count(), 0);
    }
}
