use std::sync::Arc;

use dashmap::DashMap;
use fastclaw_protocol::{AgentEvent, SessionId, TurnId, TurnSummary};
use tokio::sync::{broadcast, Mutex};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

/// Per-session actor that serialises turn execution.
///
/// Ensures at most one agent task runs per session. Incoming operations
/// are queued via `submit`. Events are broadcast to all subscribers
/// (multiple WS connections to the same session).
pub struct SessionCoordinator {
    session_id: SessionId,
    active_turn: Mutex<Option<ActiveTurn>>,
    event_tx: broadcast::Sender<AgentEvent>,
}

struct ActiveTurn {
    turn_id: TurnId,
    cancel: CancellationToken,
    task: Option<JoinHandle<anyhow::Result<TurnSummary>>>,
}

impl SessionCoordinator {
    pub fn new(session_id: SessionId) -> Self {
        let (event_tx, _) = broadcast::channel(256);
        Self {
            session_id,
            active_turn: Mutex::new(None),
            event_tx,
        }
    }

    /// Returns the session ID this coordinator manages.
    pub fn session_id(&self) -> &SessionId {
        &self.session_id
    }

    /// Subscribe to the event stream for this session.
    pub fn subscribe(&self) -> broadcast::Receiver<AgentEvent> {
        self.event_tx.subscribe()
    }

    /// Get a sender clone for emitting events into this session's broadcast.
    pub fn event_sender(&self) -> broadcast::Sender<AgentEvent> {
        self.event_tx.clone()
    }

    /// Returns true if a turn is currently running.
    pub async fn is_active(&self) -> bool {
        let mut guard = self.active_turn.lock().await;
        match guard.as_ref() {
            Some(active) => match active.task.as_ref() {
                Some(t) if t.is_finished() => {
                    guard.take();
                    false
                }
                _ => true,
            },
            None => false,
        }
    }

    /// Register a running turn with a task handle. Returns an error if a turn is already active.
    pub async fn start_turn(
        &self,
        turn_id: TurnId,
        cancel: CancellationToken,
        task: JoinHandle<anyhow::Result<TurnSummary>>,
    ) -> Result<(), String> {
        let mut guard = self.active_turn.lock().await;
        if let Some(ref active) = *guard {
            return Err(format!(
                "session {} already has active turn {}",
                self.session_id.as_str(),
                active.turn_id.as_str()
            ));
        }
        *guard = Some(ActiveTurn {
            turn_id,
            cancel,
            task: Some(task),
        });
        Ok(())
    }

    /// Register a turn without a task handle (used when the caller manages the task itself).
    pub async fn register_turn(
        &self,
        turn_id: TurnId,
        cancel: CancellationToken,
    ) -> Result<(), String> {
        let mut guard = self.active_turn.lock().await;
        if let Some(ref active) = *guard {
            return Err(format!(
                "session {} already has active turn {}",
                self.session_id.as_str(),
                active.turn_id.as_str()
            ));
        }
        *guard = Some(ActiveTurn {
            turn_id,
            cancel,
            task: None,
        });
        Ok(())
    }

    /// Cancel the currently active turn if any. Returns the turn_id if cancelled.
    pub async fn cancel_active_turn(&self) -> Option<TurnId> {
        let mut guard = self.active_turn.lock().await;
        if let Some(active) = guard.take() {
            active.cancel.cancel();
            Some(active.turn_id)
        } else {
            None
        }
    }

    /// Clear the active turn (called when a turn completes).
    pub async fn complete_turn(&self) -> Option<TurnId> {
        let mut guard = self.active_turn.lock().await;
        guard.take().map(|t| t.turn_id)
    }
}

/// Registry of active session coordinators.
///
/// Lives in `AppState`. Gateway dispatch looks up or creates a coordinator
/// per session before submitting work.
pub struct CoordinatorRegistry {
    coordinators: DashMap<SessionId, Arc<SessionCoordinator>>,
}

impl CoordinatorRegistry {
    pub fn new() -> Self {
        Self {
            coordinators: DashMap::new(),
        }
    }

    /// Get or create a coordinator for the given session.
    pub fn get_or_create(&self, session_id: &SessionId) -> Arc<SessionCoordinator> {
        self.coordinators
            .entry(session_id.clone())
            .or_insert_with(|| Arc::new(SessionCoordinator::new(session_id.clone())))
            .clone()
    }

    /// Get an existing coordinator if one exists.
    pub fn get(&self, session_id: &SessionId) -> Option<Arc<SessionCoordinator>> {
        self.coordinators.get(session_id).map(|v| v.clone())
    }

    /// Remove a coordinator (e.g., when a session is deleted).
    pub fn remove(&self, session_id: &SessionId) {
        self.coordinators.remove(session_id);
    }

    /// Number of active coordinators.
    pub fn len(&self) -> usize {
        self.coordinators.len()
    }

    pub fn is_empty(&self) -> bool {
        self.coordinators.is_empty()
    }
}

impl Default for CoordinatorRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn coordinator_enforces_single_turn() {
        let coord = SessionCoordinator::new(SessionId::new("s1"));
        let cancel1 = CancellationToken::new();
        let task1 = tokio::spawn(async {
            Ok(TurnSummary {
                turn_id: TurnId::new("t1"),
                tool_calls_made: 0,
                iterations: 0,
                usage: None,
                elapsed_ms: 0,
                context_tokens: None,
                context_window: None,
            })
        });

        coord
            .start_turn(TurnId::new("t1"), cancel1, task1)
            .await
            .unwrap();
        assert!(coord.is_active().await);

        let cancel2 = CancellationToken::new();
        let task2 = tokio::spawn(async {
            Ok(TurnSummary {
                turn_id: TurnId::new("t2"),
                tool_calls_made: 0,
                iterations: 0,
                usage: None,
                elapsed_ms: 0,
                context_tokens: None,
                context_window: None,
            })
        });

        let result = coord
            .start_turn(TurnId::new("t2"), cancel2, task2)
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn coordinator_cancel_clears_active() {
        let coord = SessionCoordinator::new(SessionId::new("s1"));
        let cancel = CancellationToken::new();
        let task = tokio::spawn(async {
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            Ok(TurnSummary {
                turn_id: TurnId::new("t1"),
                tool_calls_made: 0,
                iterations: 0,
                usage: None,
                elapsed_ms: 0,
                context_tokens: None,
                context_window: None,
            })
        });

        coord
            .start_turn(TurnId::new("t1"), cancel, task)
            .await
            .unwrap();
        let cancelled = coord.cancel_active_turn().await;
        assert_eq!(cancelled.unwrap().as_str(), "t1");
        assert!(!coord.is_active().await);
    }

    #[test]
    fn registry_get_or_create() {
        let registry = CoordinatorRegistry::new();
        let c1 = registry.get_or_create(&SessionId::new("s1"));
        let c2 = registry.get_or_create(&SessionId::new("s1"));
        assert_eq!(c1.session_id().as_str(), c2.session_id().as_str());
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn registry_remove() {
        let registry = CoordinatorRegistry::new();
        registry.get_or_create(&SessionId::new("s1"));
        assert_eq!(registry.len(), 1);
        registry.remove(&SessionId::new("s1"));
        assert!(registry.is_empty());
    }
}
