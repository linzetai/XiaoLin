use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{broadcast, RwLock};
use tracing::{debug, info};

use xiaolin_protocol::id::{SessionId, SubmissionId};

use crate::actor::{SessionActor, SessionActorConfig};
use crate::handle::{SessionHandle, SubmitError};
use crate::submission::SessionOp;
use crate::turn::TurnExecutor;

/// Statistics returned by garbage collection.
#[derive(Debug, Clone, Copy)]
pub struct GcStats {
    pub removed: usize,
    pub alive: usize,
}

/// Manages the lifecycle of session actors.
///
/// Aligned with Codex's `ThreadManager` — responsible for creating, resuming,
/// forking, and unloading sessions. Also provides the primary `submit()`
/// entry point that gateway handlers use.
pub struct SessionManager {
    state: Arc<SessionManagerState>,
}

struct SessionManagerState {
    sessions: RwLock<HashMap<SessionId, Arc<SessionHandle>>>,
    session_created_tx: broadcast::Sender<SessionId>,
    turn_executor: Arc<dyn TurnExecutor>,
    sq_capacity: usize,
}

impl SessionManager {
    pub fn new(turn_executor: Arc<dyn TurnExecutor>) -> Self {
        let (tx, _) = broadcast::channel(64);
        Self {
            state: Arc::new(SessionManagerState {
                sessions: RwLock::new(HashMap::new()),
                session_created_tx: tx,
                turn_executor,
                sq_capacity: SessionActorConfig::DEFAULT_SQ_CAPACITY,
            }),
        }
    }

    /// Create a new session manager with custom SQ capacity.
    pub fn with_capacity(turn_executor: Arc<dyn TurnExecutor>, sq_capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(64);
        Self {
            state: Arc::new(SessionManagerState {
                sessions: RwLock::new(HashMap::new()),
                session_created_tx: tx,
                turn_executor,
                sq_capacity,
            }),
        }
    }

    /// Get an existing session or create a new one.
    pub async fn get_or_create(
        &self,
        session_id: SessionId,
        agent_id: &str,
    ) -> Arc<SessionHandle> {
        // Fast path: read lock.
        {
            let sessions = self.state.sessions.read().await;
            if let Some(handle) = sessions.get(&session_id) {
                if handle.is_alive() {
                    return Arc::clone(handle);
                }
            }
        }

        // Slow path: write lock, create.
        let mut sessions = self.state.sessions.write().await;
        // Double-check after acquiring write lock.
        if let Some(handle) = sessions.get(&session_id) {
            if handle.is_alive() {
                return Arc::clone(handle);
            }
        }

        let handle = SessionActor::spawn(SessionActorConfig {
            session_id: session_id.clone(),
            agent_id: agent_id.to_string(),
            submission_queue_capacity: self.state.sq_capacity,
            turn_executor: Arc::clone(&self.state.turn_executor),
        });

        let handle = Arc::new(handle);
        sessions.insert(session_id.clone(), Arc::clone(&handle));
        let _ = self.state.session_created_tx.send(session_id.clone());
        info!(session_id = %session_id, "session actor created");

        handle
    }

    /// Get an existing session handle, if it exists and is alive.
    pub async fn get(&self, session_id: &SessionId) -> Option<Arc<SessionHandle>> {
        let sessions = self.state.sessions.read().await;
        sessions
            .get(session_id)
            .filter(|h| h.is_alive())
            .cloned()
    }

    /// Submit an operation to a specific session. Creates the session if needed.
    pub async fn submit(
        &self,
        session_id: SessionId,
        agent_id: &str,
        op: SessionOp,
    ) -> Result<SubmissionId, SubmitError> {
        let handle = self.get_or_create(session_id, agent_id).await;
        handle.submit(op).await
    }

    /// Unload a session actor (request shutdown and remove from the map).
    pub async fn unload(&self, session_id: &SessionId) {
        let handle = {
            let mut sessions = self.state.sessions.write().await;
            sessions.remove(session_id)
        };
        if let Some(handle) = handle {
            let _ = handle.submit(SessionOp::Shutdown).await;
            debug!(session_id = %session_id, "session unloaded");
        }
    }

    /// List all active session IDs.
    pub async fn active_sessions(&self) -> Vec<SessionId> {
        let sessions = self.state.sessions.read().await;
        sessions
            .iter()
            .filter(|(_, h)| h.is_alive())
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// Number of active sessions.
    pub async fn active_count(&self) -> usize {
        let sessions = self.state.sessions.read().await;
        sessions.values().filter(|h| h.is_alive()).count()
    }

    /// Remove dead sessions from the map.
    pub async fn gc(&self) {
        let mut sessions = self.state.sessions.write().await;
        let before = sessions.len();
        sessions.retain(|_, h| h.is_alive());
        let removed = before - sessions.len();
        if removed > 0 {
            debug!(removed, "garbage collected dead sessions");
        }
    }

    /// Remove dead sessions and return stats for monitoring.
    pub async fn gc_with_stats(&self) -> GcStats {
        let mut sessions = self.state.sessions.write().await;
        let before = sessions.len();
        sessions.retain(|_, h| h.is_alive());
        let removed = before - sessions.len();
        let alive = sessions.len();
        if removed > 0 {
            info!(removed, alive, "session GC: cleaned dead sessions");
        }
        GcStats { removed, alive }
    }

    /// Get all active session IDs (for use by resource cleanup).
    pub async fn active_session_id_set(&self) -> std::collections::HashSet<String> {
        let sessions = self.state.sessions.read().await;
        sessions
            .iter()
            .filter(|(_, h)| h.is_alive())
            .map(|(id, _)| id.to_string())
            .collect()
    }

    /// Gracefully shut down all sessions.
    pub async fn shutdown_all(&self) {
        let handles: Vec<Arc<SessionHandle>> = {
            let sessions = self.state.sessions.read().await;
            sessions.values().cloned().collect()
        };

        for handle in &handles {
            let _ = handle.submit(SessionOp::Shutdown).await;
        }

        for handle in &handles {
            handle.wait_until_stopped().await;
        }

        let mut sessions = self.state.sessions.write().await;
        sessions.clear();
        info!("all sessions shut down");
    }

    /// Subscribe to session creation events.
    pub fn on_session_created(&self) -> broadcast::Receiver<SessionId> {
        self.state.session_created_tx.subscribe()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::turn::{TurnError, TurnParams, TurnResult};
    use std::sync::atomic::{AtomicU32, Ordering};

    struct CountingExecutor(AtomicU32);

    #[async_trait::async_trait]
    impl TurnExecutor for CountingExecutor {
        async fn execute(
            &self,
            _params: TurnParams,
            _interaction: crate::interaction::InteractionHandle,
            _tx: tokio::sync::mpsc::Sender<xiaolin_protocol::AgentEvent>,
            _cancel: tokio_util::sync::CancellationToken,
        ) -> Result<TurnResult, TurnError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(TurnResult {
                tool_calls_made: 0,
                iterations: 1,
                usage: None,
            })
        }
    }

    fn test_manager() -> (SessionManager, Arc<CountingExecutor>) {
        let exec = Arc::new(CountingExecutor(AtomicU32::new(0)));
        let manager = SessionManager::new(exec.clone());
        (manager, exec)
    }

    #[tokio::test]
    async fn get_or_create_returns_same_handle() {
        let (mgr, _) = test_manager();
        let h1 = mgr
            .get_or_create(SessionId::new("s1"), "agent")
            .await;
        let h2 = mgr
            .get_or_create(SessionId::new("s1"), "agent")
            .await;
        // Same Arc — pointer equality.
        assert!(Arc::ptr_eq(&h1, &h2));
    }

    #[tokio::test]
    async fn different_sessions_get_different_handles() {
        let (mgr, _) = test_manager();
        let h1 = mgr
            .get_or_create(SessionId::new("s1"), "agent")
            .await;
        let h2 = mgr
            .get_or_create(SessionId::new("s2"), "agent")
            .await;
        assert!(!Arc::ptr_eq(&h1, &h2));
    }

    #[tokio::test]
    async fn active_count_tracks_sessions() {
        let (mgr, _) = test_manager();
        assert_eq!(mgr.active_count().await, 0);

        mgr.get_or_create(SessionId::new("s1"), "agent").await;
        assert_eq!(mgr.active_count().await, 1);

        mgr.get_or_create(SessionId::new("s2"), "agent").await;
        assert_eq!(mgr.active_count().await, 2);
    }

    #[tokio::test]
    async fn unload_removes_session() {
        let (mgr, _) = test_manager();
        mgr.get_or_create(SessionId::new("s1"), "agent").await;
        assert_eq!(mgr.active_count().await, 1);

        mgr.unload(&SessionId::new("s1")).await;
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        mgr.gc().await;
        assert_eq!(mgr.active_count().await, 0);
    }

    #[tokio::test]
    async fn submit_creates_session_on_demand() {
        let (mgr, exec) = test_manager();
        let result = mgr
            .submit(
                SessionId::new("auto"),
                "agent",
                SessionOp::UserTurn {
                    messages: serde_json::json!([]),
                    agent_id: None,
                    model: None,
                    work_dir: None,
                    extra: Default::default(),
                },
            )
            .await;
        assert!(result.is_ok());
        assert_eq!(mgr.active_count().await, 1);

        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        assert!(exec.0.load(Ordering::SeqCst) >= 1);

        mgr.shutdown_all().await;
    }

    #[tokio::test]
    async fn shutdown_all_clears_sessions() {
        let (mgr, _) = test_manager();
        mgr.get_or_create(SessionId::new("s1"), "agent").await;
        mgr.get_or_create(SessionId::new("s2"), "agent").await;

        mgr.shutdown_all().await;
        assert_eq!(mgr.active_count().await, 0);
    }

    #[tokio::test]
    async fn gc_with_stats_reports_alive_sessions() {
        let (mgr, _) = test_manager();
        mgr.get_or_create(SessionId::new("s1"), "agent").await;
        mgr.get_or_create(SessionId::new("s2"), "agent").await;
        assert_eq!(mgr.active_count().await, 2);

        // No dead sessions, GC should report 0 removed, 2 alive
        let stats = mgr.gc_with_stats().await;
        assert_eq!(stats.removed, 0);
        assert_eq!(stats.alive, 2);
    }

    #[tokio::test]
    async fn active_session_id_set_returns_live_ids() {
        let (mgr, _) = test_manager();
        mgr.get_or_create(SessionId::new("s1"), "agent").await;
        mgr.get_or_create(SessionId::new("s2"), "agent").await;

        let ids = mgr.active_session_id_set().await;
        assert!(ids.contains("s1"));
        assert!(ids.contains("s2"));
        assert_eq!(ids.len(), 2);

        mgr.unload(&SessionId::new("s1")).await;
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        mgr.gc().await;

        let ids = mgr.active_session_id_set().await;
        assert!(!ids.contains("s1"));
        assert!(ids.contains("s2"));
        assert_eq!(ids.len(), 1);
    }
}
