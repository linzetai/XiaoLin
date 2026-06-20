use std::sync::Arc;

use futures::Stream;

use super::agent_context::AgentContext;
use super::agent_step::AgentStep;
use super::turn_loop;
use super::turn_setup;
use super::AgentRuntime;
use crate::builtin_tools::file_state_cache::FileStateCache;

impl AgentRuntime {
    /// Execute an agent turn as a composable async stream.
    ///
    /// This is the primary execution API. The stream yields `AgentStep` events
    /// as the agent processes LLM responses and tool calls.
    ///
    /// Architecture: two channels carry events out of the spawned execution task:
    /// - `step_tx` → `step_rx`: main-loop events yielded directly as `AgentStep`
    /// - `event_tx` (caller's channel): side-path events forwarded by the tool dispatcher
    ///
    /// # Cancellation
    ///
    /// If `ctx.cancel_token` is set, the loop checks for cancellation at each
    /// iteration boundary. When cancelled, the stream emits an Error step and ends.
    /// Dropping the stream does NOT automatically cancel the spawned task — callers
    /// should cancel the token explicitly.
    pub fn execute_as_stream(
        runtime: Arc<Self>,
        ctx: AgentContext,
    ) -> impl Stream<Item = AgentStep> + Send + 'static {
        // Capture task-locals before tokio::spawn so they survive the
        // task boundary. The session_bridge wraps the call site with
        // with_session_mode / SUBAGENT_SESSION_ID / with_stream_context /
        // with_interaction_handle, but tokio::spawn creates a fresh task
        // that loses all task-locals.
        let captured_mode_state = crate::builtin_tools::current_session_mode();
        let captured_plan_ctx = crate::builtin_tools::current_plan_context();
        let captured_session_id = crate::subagent::SUBAGENT_SESSION_ID
            .try_with(|s| s.clone())
            .ok();
        let captured_stream_key = crate::builtin_tools::ASK_QUESTION_STREAM_KEY
            .try_with(|k| k.clone())
            .ok();
        let captured_interaction_handle = crate::builtin_tools::TASK_INTERACTION_HANDLE
            .try_with(|h| h.clone())
            .ok();
        let captured_steer_inbox = crate::builtin_tools::STEER_INBOX
            .try_with(|s| s.clone())
            .ok();

        async_stream::stream! {
            let (step_tx, mut step_rx) = tokio::sync::mpsc::channel::<AgentStep>(512);

            let handle = tokio::spawn(async move {
                let mut ctx = ctx;
                ctx.step_tx = Some(step_tx);

                let turn_fut = async {
                    let (mut ms, svc) = turn_setup::setup_turn(runtime, &ctx).await?;
                    turn_loop::run_turn_loop(&mut ms, &svc).await
                };

                // Re-establish task-locals captured from the parent task.
                let with_mode = if let Some(ms) = captured_mode_state {
                    futures::future::Either::Left(
                        crate::builtin_tools::with_session_mode(ms, captured_plan_ctx, turn_fut),
                    )
                } else {
                    futures::future::Either::Right(turn_fut)
                };
                let file_cache = Arc::new(FileStateCache::new());
                let with_fsc = crate::builtin_tools::with_file_state_cache(
                    file_cache,
                    with_mode,
                );
                let with_sid = if let Some(sid) = captured_session_id {
                    futures::future::Either::Left(
                        crate::subagent::SUBAGENT_SESSION_ID.scope(sid, with_fsc),
                    )
                } else {
                    futures::future::Either::Right(with_fsc)
                };
                let with_ih = if let Some(ih) = captured_interaction_handle {
                    futures::future::Either::Left(
                        crate::builtin_tools::TASK_INTERACTION_HANDLE.scope(ih, with_sid),
                    )
                } else {
                    futures::future::Either::Right(with_sid)
                };
                let with_steer = if let Some(inbox) = captured_steer_inbox {
                    futures::future::Either::Left(
                        crate::builtin_tools::STEER_INBOX.scope(inbox, with_ih),
                    )
                } else {
                    futures::future::Either::Right(with_ih)
                };
                if let Some(key) = captured_stream_key {
                    crate::builtin_tools::ASK_QUESTION_STREAM_KEY.scope(key, with_steer).await
                } else {
                    with_steer.await
                }
            });

            // Abort the spawned task on stream drop to prevent resource leaks
            // (leaked LLM calls, tool side-effects, etc.).
            let abort_handle = handle.abort_handle();
            struct AbortOnDrop(tokio::task::AbortHandle);
            impl Drop for AbortOnDrop {
                fn drop(&mut self) {
                    if !self.0.is_finished() {
                        self.0.abort();
                    }
                }
            }
            let _abort_guard = AbortOnDrop(abort_handle);

            while let Some(step) = step_rx.recv().await {
                yield step;
            }

            match handle.await {
                Ok(Ok(_summary)) => {
                    // TurnEnd was already emitted via step_tx and yielded above.
                }
                Ok(Err(e)) => {
                    yield AgentStep::Error {
                        turn_id: xiaolin_protocol::TurnId::generate(),
                        message: e.to_string(),
                        error_code: None,
                        recoverable: false,
                    };
                }
                Err(join_err) if join_err.is_cancelled() => {
                    // Task was aborted by our guard — not a panic.
                }
                Err(join_err) => {
                    yield AgentStep::Error {
                        turn_id: xiaolin_protocol::TurnId::generate(),
                        message: format!("execution task panicked: {join_err}"),
                        error_code: None,
                        recoverable: false,
                    };
                }
            }
        }
    }
}
