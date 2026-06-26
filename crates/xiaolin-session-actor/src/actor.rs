use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use parking_lot::Mutex;

use tokio::sync::{watch, Notify};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, info_span, warn, Instrument};

use xiaolin_protocol::approval::ApprovalDecision;
use xiaolin_protocol::event::AbortReason;
use xiaolin_protocol::id::{SessionId, SubmissionId, TurnId};
use xiaolin_protocol::AgentEvent;

use crate::fanout::{EventFanout, SharedFanout};
use crate::interaction::{InteractionRegistrar, TurnInteractionPort};
use crate::submission::{SessionEvent, SessionOp, Submission};
use crate::turn::{ActiveTurn, TurnExecutor};

/// Status of a session actor, observable via `watch` channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentStatus {
    /// No active turn; waiting for submissions.
    Idle,
    /// A turn is executing.
    Running,
    /// The actor is shutting down.
    ShuttingDown,
}

/// Configuration for creating a session actor.
pub struct SessionActorConfig {
    pub session_id: SessionId,
    pub agent_id: String,
    pub submission_queue_capacity: usize,
    pub turn_executor: Arc<dyn TurnExecutor>,
}

impl SessionActorConfig {
    /// Bounded SQ capacity. Aligned with Codex's 512 but we default lower
    /// since XiaoLin sessions are typically lighter.
    pub const DEFAULT_SQ_CAPACITY: usize = 256;
    /// Default buffer size for session event subscribers.
    pub const DEFAULT_SUBSCRIBER_BUFFER: usize = 1024;
}

/// The session actor — one per session, owns the SQ/EQ channel pair.
///
/// All control operations are serialized through the actor's submission loop.
/// Heavy work (turn execution) is offloaded to tokio tasks. Approvals block
/// inside the task, not the actor loop (Codex's core invariant).
pub struct SessionActor {
    session_id: SessionId,
    agent_id: String,
    rx_sub: async_channel::Receiver<Submission>,
    fanout: SharedFanout,
    interaction_port: TurnInteractionPort,
    active_turn: Option<ActiveTurn>,
    registrar: Option<InteractionRegistrar>,
    status_tx: watch::Sender<AgentStatus>,
    turn_executor: Arc<dyn TurnExecutor>,
    cancellation_token: CancellationToken,
    /// Per-session approval cache. `ApprovedForSession` decisions are stored
    /// here so they only affect this session, not other sessions sharing the
    /// same `ToolOrchestrator`. The map key is the action cache key.
    session_approvals: Arc<std::sync::Mutex<HashMap<String, ApprovalDecision>>>,
    /// Sender for mid-turn steer inputs. Created when a turn starts and
    /// dropped when the turn ends or is aborted.
    steer_tx: Option<tokio::sync::mpsc::UnboundedSender<crate::turn::SteerMessage>>,
}

impl SessionActor {
    /// Spawn a new session actor. Returns a handle for submitting ops and
    /// receiving events.
    pub fn spawn(config: SessionActorConfig) -> crate::handle::SessionHandle {
        let (tx_sub, rx_sub) =
            async_channel::bounded::<Submission>(config.submission_queue_capacity);
        let (status_tx, status_rx) = watch::channel(AgentStatus::Idle);
        let cancellation_token = CancellationToken::new();
        let fanout = Arc::new(Mutex::new(EventFanout::new()));

        let session_approvals = Arc::new(std::sync::Mutex::new(HashMap::new()));

        let actor = Self {
            session_id: config.session_id.clone(),
            agent_id: config.agent_id.clone(),
            rx_sub,
            fanout: fanout.clone(),
            interaction_port: TurnInteractionPort::new(),
            active_turn: None,
            registrar: None,
            status_tx,
            turn_executor: config.turn_executor,
            cancellation_token: cancellation_token.clone(),
            session_approvals,
            steer_tx: None,
        };

        let session_id = config.session_id.clone();
        let task_handle = tokio::spawn(
            actor
                .run()
                .instrument(info_span!("session_actor", session_id = %session_id)),
        );

        crate::handle::SessionHandle::new(
            config.session_id,
            config.agent_id,
            tx_sub,
            status_rx,
            cancellation_token,
            task_handle,
            fanout,
        )
    }

    /// The actor loop — single consumer of the submission queue.
    async fn run(mut self) {
        info!(session_id = %self.session_id, "session actor started");

        loop {
            // Drain any pending interaction registrations from the active turn task.
            if let Some(ref mut reg) = self.registrar {
                reg.drain_into(&mut self.interaction_port);
            }

            // Periodically GC closed fanout subscribers.
            {
                let mut f = self.fanout.lock();
                f.gc();
            }

            if let Ok(mut approvals) = self.session_approvals.lock() {
                crate::turn::prune_session_approvals_if_needed(&mut approvals);
            }

            tokio::select! {
                sub = self.rx_sub.recv() => {
                    match sub {
                        Ok(sub) => {
                            if self.dispatch(sub).await {
                                break;
                            }
                        }
                        Err(_) => {
                            debug!(session_id = %self.session_id, "SQ closed, shutting down");
                            break;
                        }
                    }
                }
                () = self.cancellation_token.cancelled() => {
                    debug!(session_id = %self.session_id, "cancellation requested");
                    break;
                }
            }
        }

        // Teardown: abort active turn, update status.
        if self.active_turn.is_some() {
            self.abort_active_turn(AbortReason::Interrupted).await;
        }
        let _ = self.status_tx.send(AgentStatus::ShuttingDown);
        info!(session_id = %self.session_id, "session actor stopped");
    }

    /// Dispatch a single submission. Returns `true` if the actor should exit.
    async fn dispatch(&mut self, sub: Submission) -> bool {
        match sub.op {
            SessionOp::UserTurn {
                messages,
                agent_id,
                model,
                work_dir,
                extra,
                typed_data,
            } => {
                self.handle_user_turn(
                    sub.id, messages, agent_id, model, work_dir, extra, typed_data,
                )
                .await;
                false
            }
            SessionOp::Interrupt => {
                self.handle_interrupt(sub.id).await;
                false
            }
            SessionOp::ResolveApproval {
                interaction_id,
                decision,
            } => {
                // Drain registrations first so we see the latest pending items.
                if let Some(ref mut reg) = self.registrar {
                    reg.drain_into(&mut self.interaction_port);
                }
                if !self
                    .interaction_port
                    .resolve_approval(&interaction_id, decision)
                {
                    warn!(interaction_id, "no pending approval found");
                }
                false
            }
            SessionOp::ResolveAnswer {
                interaction_id,
                answer,
            } => {
                if let Some(ref mut reg) = self.registrar {
                    reg.drain_into(&mut self.interaction_port);
                }
                if !self
                    .interaction_port
                    .resolve_answer(&interaction_id, answer)
                {
                    warn!(interaction_id, "no pending answer found");
                }
                false
            }
            SessionOp::SteerInput { messages } => {
                if let Some(ref steer_tx) = self.steer_tx {
                    for msg in messages {
                        if steer_tx.send(msg).is_err() {
                            warn!("steer_tx closed; turn may have already ended");
                            break;
                        }
                    }
                } else {
                    warn!(sub_id = %sub.id, "SteerInput received but no active turn");
                }
                false
            }
            SessionOp::Compact => {
                self.handle_compact(sub.id).await;
                false
            }
            SessionOp::ForkSession { .. } => {
                self.emit_event(
                    &sub.id,
                    AgentEvent::Error {
                        turn_id: TurnId::new("fork"),
                        message: "ForkSession is not yet implemented".into(),
                        error_code: Some(xiaolin_protocol::event::ErrorCode::BadRequest),
                    },
                )
                .await;
                false
            }
            SessionOp::RollbackTurns { .. } => {
                self.emit_event(
                    &sub.id,
                    AgentEvent::Error {
                        turn_id: TurnId::new("rollback"),
                        message: "RollbackTurns is not yet implemented".into(),
                        error_code: Some(xiaolin_protocol::event::ErrorCode::BadRequest),
                    },
                )
                .await;
                false
            }
            SessionOp::UpdateSettings { .. } => {
                debug!("UpdateSettings: per-session settings update accepted");
                false
            }
            SessionOp::Shutdown => {
                self.handle_shutdown(sub.id).await;
                true
            }
        }
    }

    /// Start a new user turn. Aborts any active turn first (Codex invariant).
    #[allow(clippy::too_many_arguments)]
    async fn handle_user_turn(
        &mut self,
        sub_id: SubmissionId,
        messages: serde_json::Value,
        agent_id: Option<String>,
        model: Option<String>,
        work_dir: Option<String>,
        extra: serde_json::Map<String, serde_json::Value>,
        typed_data: Option<std::sync::Arc<dyn std::any::Any + Send + Sync>>,
    ) {
        // 1. Abort any active turn.
        if self.active_turn.is_some() {
            self.abort_active_turn(AbortReason::Replaced).await;
        }

        // 2. Create turn context.
        let turn_id = TurnId::generate();
        let _ = self.status_tx.send(AgentStatus::Running);

        // 3. Emit TurnStart.
        self.emit_event(
            &sub_id,
            AgentEvent::TurnStart {
                turn_id: turn_id.clone(),
                session_id: Some(self.session_id.to_string()),
                execution_mode: None,
                requested_execution_mode: None,
                mode_source: None,
            },
        )
        .await;

        // 4. Create interaction channel for this turn.
        let (interaction_handle, registrar) = crate::interaction::interaction_channel();
        self.registrar = Some(registrar);

        // 5. Spawn the turn task.
        let cancel_token = CancellationToken::new();
        let done = Arc::new(Notify::new());

        let task_turn_id = turn_id.clone();
        let task_session_id = self.session_id.clone();
        let task_agent_id = agent_id.unwrap_or_else(|| self.agent_id.clone());
        let executor = Arc::clone(&self.turn_executor);
        let task_done = Arc::clone(&done);
        let task_cancel = cancel_token.clone();

        // Build a bounded event sender for the turn task → fanout subscribers.
        let (task_tx, mut task_rx) = tokio::sync::mpsc::channel::<AgentEvent>(64);

        // Relay task: forward events from turn task to all fanout subscribers.
        let relay_session_id = self.session_id.clone();
        let relay_sub_id = sub_id.clone();
        let relay_fanout = self.fanout.clone();
        let relay_handle = tokio::spawn(async move {
            while let Some(event) = task_rx.recv().await {
                let session_event = SessionEvent {
                    id: relay_sub_id.clone(),
                    session_id: relay_session_id.clone(),
                    msg: event,
                };
                let senders = {
                    let f = relay_fanout.lock();
                    f.subscriber_senders()
                };
                for tx in &senders {
                    match tokio::time::timeout(
                        std::time::Duration::from_secs(5),
                        tx.send(session_event.clone()),
                    )
                    .await
                    {
                        Ok(Ok(())) => {}
                        Ok(Err(_closed)) => {}
                        Err(_timeout) => {
                            tracing::warn!(
                                session_id = %relay_session_id,
                                "relay send timed out after 5s, dropping event for slow subscriber"
                            );
                        }
                    }
                }
            }
        });

        let turn_start = Instant::now();
        let task_approval_cache = self.session_approvals.clone();
        let (steer_tx, steer_rx) = tokio::sync::mpsc::unbounded_channel();
        self.steer_tx = Some(steer_tx);
        let handle = tokio::spawn(async move {
            let params = crate::turn::TurnParams {
                session_id: task_session_id.clone(),
                turn_id: task_turn_id.clone(),
                agent_id: task_agent_id,
                messages,
                model,
                work_dir,
                extra,
                approval_cache: task_approval_cache,
                steer_rx,
                typed_data,
            };

            let outcome = executor
                .execute(params, interaction_handle, task_tx.clone(), task_cancel)
                .await;

            let elapsed = turn_start.elapsed();

            match outcome {
                Ok(result) => {
                    let summary = xiaolin_protocol::event::TurnSummary {
                        turn_id: task_turn_id.clone(),
                        tool_calls_made: result.tool_calls_made,
                        iterations: result.iterations,
                        usage: result.usage,
                        elapsed_ms: elapsed.as_millis() as u64,
                        context_tokens: None,
                        context_window: None,
                    };
                    let _ = task_tx
                        .send(AgentEvent::TurnEnd {
                            turn_id: task_turn_id,
                            summary,
                            session_id: Some(task_session_id.to_string()),
                            final_tool_calls: None,
                            reason: None,
                            diagnosis: None,
                            plan_outcome: None,
                        })
                        .await;
                }
                Err(crate::turn::TurnError::Cancelled) => {
                    // Actor already emits TurnAborted; no extra error event.
                }
                Err(ref turn_err) if turn_err.affects_turn_status() => {
                    let code = match turn_err {
                        crate::turn::TurnError::Runtime { code, .. } => Some(*code),
                        crate::turn::TurnError::Cancelled => None,
                    };
                    let _ = task_tx
                        .send(AgentEvent::Error {
                            turn_id: task_turn_id.clone(),
                            message: turn_err.to_string(),
                            error_code: code,
                        })
                        .await;
                    let summary = xiaolin_protocol::event::TurnSummary {
                        turn_id: task_turn_id.clone(),
                        tool_calls_made: 0,
                        iterations: 0,
                        usage: None,
                        elapsed_ms: elapsed.as_millis() as u64,
                        context_tokens: None,
                        context_window: None,
                    };
                    let _ = task_tx
                        .send(AgentEvent::TurnEnd {
                            turn_id: task_turn_id,
                            summary,
                            session_id: Some(task_session_id.to_string()),
                            final_tool_calls: None,
                            reason: None,
                            diagnosis: None,
                            plan_outcome: None,
                        })
                        .await;
                }
                Err(ref turn_err) => {
                    let code = match turn_err {
                        crate::turn::TurnError::Runtime { code, .. } => Some(*code),
                        crate::turn::TurnError::Cancelled => None,
                    };
                    let _ = task_tx
                        .send(AgentEvent::Error {
                            turn_id: task_turn_id.clone(),
                            message: turn_err.to_string(),
                            error_code: code,
                        })
                        .await;
                }
            }

            task_done.notify_one();
        });

        // 6. Register active turn.
        self.active_turn = Some(ActiveTurn {
            sub_id,
            turn_id,
            handle: tokio_util::task::AbortOnDropHandle::new(handle),
            cancel_token,
            done,
            relay_handle,
        });
    }

    /// Abort the active turn with a reason.
    async fn abort_active_turn(&mut self, reason: AbortReason) {
        if let Some(turn) = self.active_turn.take() {
            // 1. Request cooperative cancellation.
            turn.cancel_token.cancel();

            // 2. Grace period (100ms, aligned with Codex).
            tokio::select! {
                () = turn.done.notified() => {}
                () = tokio::time::sleep(std::time::Duration::from_millis(100)) => {
                    warn!(turn_id = %turn.turn_id, "turn did not stop within grace period, aborting");
                }
            }

            // 3. Hard abort (AbortOnDropHandle drops the JoinHandle).
            turn.relay_handle.abort();
            drop(turn.handle);

            // 4. Emit TurnAborted (before clearing pending, so subscribers
            //    learn about the abort while interactions are still tracked).
            self.emit_event(
                &turn.sub_id,
                AgentEvent::TurnAborted {
                    turn_id: turn.turn_id,
                    reason,
                    completed_at: None,
                    duration_ms: None,
                },
            )
            .await;

            // 5. Clear pending interactions and steer channel for this turn.
            self.interaction_port.cancel_all();
            self.registrar = None;
            self.steer_tx = None;

            let _ = self.status_tx.send(AgentStatus::Idle);
        }
    }

    /// Handle a Compact operation by spawning a turn task with `_compact: true`.
    async fn handle_compact(&mut self, sub_id: SubmissionId) {
        if self.active_turn.is_some() {
            warn!(sub_id = %sub_id, "Compact rejected: a turn is already active");
            self.emit_event(
                &sub_id,
                AgentEvent::Error {
                    turn_id: TurnId::new("compact"),
                    message: "cannot compact while a turn is active".into(),
                    error_code: Some(xiaolin_protocol::event::ErrorCode::BadRequest),
                },
            )
            .await;
            return;
        }

        let mut extra = serde_json::Map::new();
        extra.insert("_compact".into(), serde_json::Value::Bool(true));

        self.handle_user_turn(sub_id, serde_json::json!([]), None, None, None, extra, None)
            .await;
    }

    async fn handle_interrupt(&mut self, sub_id: SubmissionId) {
        if self.active_turn.is_some() {
            self.abort_active_turn(AbortReason::Interrupted).await;
        } else {
            debug!(sub_id = %sub_id, "interrupt received but no active turn");
        }
    }

    async fn handle_shutdown(&mut self, sub_id: SubmissionId) {
        if self.active_turn.is_some() {
            self.abort_active_turn(AbortReason::Interrupted).await;
        }
        let _ = self.status_tx.send(AgentStatus::ShuttingDown);
        debug!(sub_id = %sub_id, "shutdown complete");
    }

    /// Emit an event to all subscribers.
    async fn emit_event(&self, sub_id: &SubmissionId, msg: AgentEvent) {
        let event = SessionEvent {
            id: sub_id.clone(),
            session_id: self.session_id.clone(),
            msg,
        };
        let is_lifecycle = matches!(
            event.msg,
            AgentEvent::TurnStart { .. }
                | AgentEvent::TurnEnd { .. }
                | AgentEvent::TurnAborted { .. }
        );
        let senders = {
            let f = self.fanout.lock();
            f.subscriber_senders()
        };
        let mut had_closed = false;
        for tx in &senders {
            if tx.is_closed() {
                had_closed = true;
                continue;
            }
            if is_lifecycle {
                match tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    tx.send(event.clone()),
                )
                .await
                {
                    Ok(Ok(())) => {}
                    Ok(Err(_)) if tx.is_closed() => {
                        had_closed = true;
                    }
                    Ok(Err(e)) => {
                        warn!(
                            session_id = %self.session_id,
                            error = %e,
                            "emit_event: failed to deliver lifecycle event"
                        );
                    }
                    Err(_) => {
                        warn!(
                            session_id = %self.session_id,
                            "emit_event: timed out delivering lifecycle event to subscriber"
                        );
                    }
                }
            } else if let Err(e) = tx.try_send(event.clone()) {
                if tx.is_closed() {
                    had_closed = true;
                } else {
                    warn!(
                        session_id = %self.session_id,
                        error = %e,
                        "emit_event: subscriber channel full, event dropped"
                    );
                }
            }
        }
        if had_closed {
            let mut f = self.fanout.lock();
            f.gc();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::turn::{TurnError, TurnParams, TurnResult};
    use std::sync::atomic::{AtomicU32, Ordering};

    struct MockExecutor {
        call_count: AtomicU32,
    }

    impl MockExecutor {
        fn new() -> Self {
            Self {
                call_count: AtomicU32::new(0),
            }
        }
    }

    #[async_trait::async_trait]
    impl TurnExecutor for MockExecutor {
        async fn execute(
            &self,
            params: TurnParams,
            _interaction: crate::interaction::InteractionHandle,
            tx: tokio::sync::mpsc::Sender<AgentEvent>,
            _cancel: CancellationToken,
        ) -> Result<TurnResult, TurnError> {
            self.call_count.fetch_add(1, Ordering::SeqCst);
            let _ = tx
                .send(AgentEvent::ContentDelta {
                    turn_id: params.turn_id,
                    delta: serde_json::json!({"content": "hello"}),
                    raw_bytes: None,
                })
                .await;
            Ok(TurnResult {
                tool_calls_made: 0,
                iterations: 1,
                usage: None,
            })
        }
    }

    fn spawn_test_actor() -> (crate::handle::SessionHandle, Arc<MockExecutor>) {
        let executor = Arc::new(MockExecutor::new());
        let handle = SessionActor::spawn(SessionActorConfig {
            session_id: SessionId::new("test-sess"),
            agent_id: "test-agent".into(),
            submission_queue_capacity: 16,
            turn_executor: executor.clone(),
        });
        (handle, executor)
    }

    #[tokio::test]
    async fn actor_starts_and_stops() {
        let (handle, _exec) = spawn_test_actor();
        assert!(handle.is_alive());

        handle.submit(SessionOp::Shutdown).await.unwrap();
        handle.wait_until_stopped().await;
    }

    #[tokio::test]
    async fn user_turn_executes() {
        let (handle, exec) = spawn_test_actor();

        handle
            .submit(SessionOp::UserTurn {
                messages: serde_json::json!([{"role": "user", "content": "hi"}]),
                agent_id: None,
                model: None,
                work_dir: None,
                extra: Default::default(),
                typed_data: None,
            })
            .await
            .unwrap();

        // Wait for the turn to complete.
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        assert_eq!(exec.call_count.load(Ordering::SeqCst), 1);

        handle.submit(SessionOp::Shutdown).await.unwrap();
        handle.wait_until_stopped().await;
    }

    #[tokio::test]
    async fn interrupt_aborts_turn() {
        let (handle, _exec) = spawn_test_actor();

        handle
            .submit(SessionOp::UserTurn {
                messages: serde_json::json!([]),
                agent_id: None,
                model: None,
                work_dir: None,
                extra: Default::default(),
                typed_data: None,
            })
            .await
            .unwrap();

        // Small delay to let the turn start.
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        handle.submit(SessionOp::Interrupt).await.unwrap();
        // Allow time for abort.
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        assert!(handle.is_alive());

        handle.submit(SessionOp::Shutdown).await.unwrap();
        handle.wait_until_stopped().await;
    }

    #[tokio::test]
    async fn new_turn_replaces_previous() {
        let (handle, exec) = spawn_test_actor();

        // First turn
        handle
            .submit(SessionOp::UserTurn {
                messages: serde_json::json!([]),
                agent_id: None,
                model: None,
                work_dir: None,
                extra: Default::default(),
                typed_data: None,
            })
            .await
            .unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // Second turn — should abort the first.
        handle
            .submit(SessionOp::UserTurn {
                messages: serde_json::json!([]),
                agent_id: None,
                model: None,
                work_dir: None,
                extra: Default::default(),
                typed_data: None,
            })
            .await
            .unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        assert_eq!(exec.call_count.load(Ordering::SeqCst), 2);

        handle.submit(SessionOp::Shutdown).await.unwrap();
        handle.wait_until_stopped().await;
    }

    #[tokio::test]
    async fn submit_and_subscribe_receives_events() {
        let (handle, _exec) = spawn_test_actor();

        let (_sub_id, mut rx) = handle
            .submit_and_subscribe(
                SessionOp::UserTurn {
                    messages: serde_json::json!([{"role": "user", "content": "hi"}]),
                    agent_id: None,
                    model: None,
                    work_dir: None,
                    extra: Default::default(),
                    typed_data: None,
                },
                64,
            )
            .await
            .unwrap();

        // Collect events with a timeout.
        let mut event_types = Vec::new();
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            match tokio::time::timeout_at(deadline, rx.recv()).await {
                Ok(Some(ev)) => {
                    let ty = match &ev.msg {
                        AgentEvent::TurnStart { .. } => "TurnStart",
                        AgentEvent::ContentDelta { .. } => "ContentDelta",
                        AgentEvent::TurnEnd { .. } => "TurnEnd",
                        _ => "other",
                    };
                    event_types.push(ty);
                    if ty == "TurnEnd" {
                        break;
                    }
                }
                _ => break,
            }
        }

        assert!(
            event_types.contains(&"TurnStart"),
            "expected TurnStart in {event_types:?}",
        );
        assert!(
            event_types.contains(&"ContentDelta"),
            "expected ContentDelta in {event_types:?}",
        );
        assert!(
            event_types.contains(&"TurnEnd"),
            "expected TurnEnd in {event_types:?}",
        );

        handle.submit(SessionOp::Shutdown).await.unwrap();
        handle.wait_until_stopped().await;
    }

    // ─── Lifecycle events use blocking send with timeout ───────────────

    #[tokio::test]
    async fn lifecycle_events_delivered_even_on_tiny_subscriber_buffer() {
        let (handle, _exec) = spawn_test_actor();

        let (_sub_id, mut rx) = handle
            .submit_and_subscribe(
                SessionOp::UserTurn {
                    messages: serde_json::json!([{"role": "user", "content": "hi"}]),
                    agent_id: None,
                    model: None,
                    work_dir: None,
                    extra: Default::default(),
                    typed_data: None,
                },
                1,
            )
            .await
            .unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        let mut received = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            received.push(ev);
        }

        assert!(
            received
                .iter()
                .any(|ev| matches!(ev.msg, AgentEvent::TurnStart { .. })),
            "TurnStart must not be dropped on a full subscriber channel"
        );

        handle.submit(SessionOp::Shutdown).await.unwrap();
        handle.wait_until_stopped().await;
    }
}
