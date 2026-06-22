use tokio::sync::{mpsc, watch};
use tokio_util::sync::CancellationToken;

use xiaolin_protocol::id::{SessionId, SubmissionId};
use xiaolin_protocol::Envelope;

use crate::actor::AgentStatus;
use crate::fanout::{BackpressurePolicy, SharedFanout};
use crate::submission::{SessionEvent, SessionOp, Submission};

/// Lightweight, cloneable handle to a session actor.
///
/// Aligned with Codex's `CodexThread` — a thin wrapper exposing `submit()`
/// and event subscription. All real work happens inside the actor loop.
#[derive(Clone)]
pub struct SessionHandle {
    session_id: SessionId,
    agent_id: String,
    tx_sub: async_channel::Sender<Submission>,
    status_rx: watch::Receiver<AgentStatus>,
    cancellation_token: CancellationToken,
    fanout: SharedFanout,
    _task_handle: std::sync::Arc<tokio::task::JoinHandle<()>>,
}

impl SessionHandle {
    pub(crate) fn new(
        session_id: SessionId,
        agent_id: String,
        tx_sub: async_channel::Sender<Submission>,
        status_rx: watch::Receiver<AgentStatus>,
        cancellation_token: CancellationToken,
        task_handle: tokio::task::JoinHandle<()>,
        fanout: SharedFanout,
    ) -> Self {
        Self {
            session_id,
            agent_id,
            tx_sub,
            status_rx,
            cancellation_token,
            fanout,
            _task_handle: std::sync::Arc::new(task_handle),
        }
    }

    /// The session ID this handle is bound to.
    pub fn session_id(&self) -> &SessionId {
        &self.session_id
    }

    /// The agent ID this session is using.
    pub fn agent_id(&self) -> &str {
        &self.agent_id
    }

    /// Whether the session actor is still running.
    pub fn is_alive(&self) -> bool {
        !self.tx_sub.is_closed()
    }

    /// Current agent status.
    pub fn status(&self) -> AgentStatus {
        *self.status_rx.borrow()
    }

    /// Submit an operation to the session actor. Returns the auto-generated
    /// submission ID for event correlation.
    pub async fn submit(&self, op: SessionOp) -> Result<SubmissionId, SubmitError> {
        let sub = Submission::new(op);
        let id = sub.id.clone();
        self.tx_sub
            .send(sub)
            .await
            .map_err(|_| SubmitError::SessionDied)?;
        Ok(id)
    }

    /// Subscribe to all session events via the actor's EventFanout.
    ///
    /// Returns a subscriber ID (for later `unsubscribe`) and the receiving end.
    /// Uses `BackpressurePolicy::Drop` — full buffers silently discard non-lifecycle events.
    pub fn subscribe(&self, buffer: usize) -> (u64, mpsc::Receiver<SessionEvent>) {
        let mut f = self.fanout.lock();
        f.subscribe(buffer, BackpressurePolicy::Drop)
    }

    /// Subscribe with the default buffer size (`DEFAULT_SUBSCRIBER_BUFFER`).
    pub fn subscribe_default(&self) -> (u64, mpsc::Receiver<SessionEvent>) {
        self.subscribe(crate::actor::SessionActorConfig::DEFAULT_SUBSCRIBER_BUFFER)
    }

    /// Remove a previously registered subscriber.
    pub fn unsubscribe(&self, subscriber_id: u64) {
        let mut f = self.fanout.lock();
        f.unsubscribe(subscriber_id);
    }

    /// Subscribe first, then submit. Guarantees the subscriber is registered
    /// before the actor processes the op, so no events are missed.
    pub async fn submit_and_subscribe(
        &self,
        op: SessionOp,
        buffer: usize,
    ) -> Result<(SubmissionId, mpsc::Receiver<SessionEvent>), SubmitError> {
        let (_sub_id, rx) = self.subscribe(buffer);

        let sub = Submission::new(op);
        let id = sub.id.clone();
        self.tx_sub
            .send(sub)
            .await
            .map_err(|_| SubmitError::SessionDied)?;

        Ok((id, rx))
    }

    /// Wait for the status to change.
    pub async fn wait_for_status_change(&mut self) -> AgentStatus {
        let _ = self.status_rx.changed().await;
        *self.status_rx.borrow()
    }

    /// Wait until the session actor stops (for graceful shutdown).
    pub async fn wait_until_stopped(&self) {
        let mut rx = self.status_rx.clone();
        loop {
            if self.tx_sub.is_closed() {
                return;
            }
            if rx.changed().await.is_err() {
                return;
            }
            if *rx.borrow() == AgentStatus::ShuttingDown {
                // Give the actor a moment to fully exit.
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                return;
            }
        }
    }

    /// Submit an operation using an `Envelope`, preserving the caller-assigned
    /// submission ID for end-to-end correlation.
    pub async fn submit_envelope(
        &self,
        envelope: Envelope<SessionOp>,
    ) -> Result<SubmissionId, SubmitError> {
        let sub = Submission {
            id: envelope.id,
            op: envelope.payload,
        };
        let id = sub.id.clone();
        self.tx_sub
            .send(sub)
            .await
            .map_err(|_| SubmitError::SessionDied)?;
        Ok(id)
    }

    /// Request graceful shutdown of the session actor.
    pub fn cancel(&self) {
        self.cancellation_token.cancel();
    }
}

/// Error returned when submitting to a dead session.
#[derive(Debug, thiserror::Error)]
pub enum SubmitError {
    #[error("session actor has died")]
    SessionDied,
}
