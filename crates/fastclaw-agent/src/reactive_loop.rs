//! Supervised Reactive Loop for sub-agent orchestration.
//!
//! After `execute_unified` completes a single LLM turn, this module checks for
//! active sub-agents. If any exist, it waits for completions (with batch window),
//! injects notifications into the conversation, and re-prompts the LLM.

use std::time::Duration;

use fastclaw_core::types::{ChatMessage, Role};
use fastclaw_protocol::{AgentEvent, CompletionSummary, TurnId, TurnSummary};
use tokio::sync::{broadcast, mpsc};

/// Outcome of the reactive loop: accumulated summaries from all inner execute_unified calls.
#[derive(Debug)]
pub struct ReactiveLoopResult {
    pub total_tool_calls: u32,
    pub total_iterations: u32,
    pub reprompt_count: u32,
}

/// Build the system message notification text for completed sub-agents.
pub fn build_completion_notification(
    completions: &[CompletionSummary],
    remaining_active: usize,
) -> String {
    let mut msg = String::with_capacity(2048);

    for summary in completions {
        msg.push_str(&format!(
            "\n[Sub-Agent Completed: {}]\n\
             Type: {} | Task: \"{}\"\n\
             Status: {} | Duration: {:.1}s | Tool calls: {}\n",
            summary.run_id,
            summary.subagent_type,
            summary.task,
            summary.status,
            summary.elapsed_ms as f64 / 1000.0,
            summary.tool_call_count,
        ));

        if let Some(ref result) = summary.result_preview {
            msg.push_str(&format!("\nResult:\n{result}\n"));
        }
        if let Some(ref error) = summary.error {
            msg.push_str(&format!("\nError: {error}\n"));
        }
        msg.push('\n');
    }

    if remaining_active > 0 {
        msg.push_str(&format!(
            "Remaining active sub-agents: {remaining_active}\n"
        ));
    } else {
        msg.push_str("All sub-agents have completed.\n");
    }

    msg.push_str(
        "\nInstruction: Process these results. You may spawn additional tasks, \
         reason about findings, or produce your final response.",
    );

    msg
}

/// Build a `ChatMessage` from a completion notification.
pub fn notification_as_system_message(text: &str) -> ChatMessage {
    ChatMessage {
        role: Role::System,
        content: Some(serde_json::Value::String(text.to_string())),
        reasoning_content: None,
        name: Some("subagent_harness".to_string()),
        tool_calls: None,
        tool_call_id: None,
        compact_metadata: None,
    }
}

/// Wait for sub-agent completions with a batch window.
///
/// Returns a non-empty vec of completions (waits for at least one, then collects
/// any others that arrive within `batch_window`).
pub async fn wait_for_completions(
    rx: &mut broadcast::Receiver<CompletionSummary>,
    batch_window: Duration,
) -> Vec<CompletionSummary> {
    let mut completions = Vec::new();

    // Wait for the first completion (blocking).
    match rx.recv().await {
        Ok(summary) => completions.push(summary),
        Err(broadcast::error::RecvError::Closed) => return completions,
        Err(broadcast::error::RecvError::Lagged(n)) => {
            tracing::warn!(lagged = n, "reactive loop completion receiver lagged");
            // Try again immediately
            if let Ok(summary) = rx.recv().await {
                completions.push(summary);
            } else {
                return completions;
            }
        }
    }

    // Batch window: collect any additional completions within the window.
    if !batch_window.is_zero() {
        let deadline = tokio::time::Instant::now() + batch_window;
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                break;
            }
            tokio::select! {
                result = rx.recv() => {
                    match result {
                        Ok(summary) => completions.push(summary),
                        Err(_) => break,
                    }
                }
                _ = tokio::time::sleep(remaining) => break,
            }
        }
    }

    completions
}

/// Determines if the LLM response in a re-prompt turn is just an "ack" (no tool calls,
/// only text) that should be suppressed.
pub fn is_intermediate_ack(summary: &TurnSummary) -> bool {
    summary.tool_calls_made == 0
}

/// Emit a `SubAgentNotification` event to the stream for frontend visibility.
pub async fn emit_notification_event(
    tx: &mpsc::Sender<AgentEvent>,
    turn_id: &TurnId,
    completions: &[CompletionSummary],
    remaining_active: u32,
) {
    let _ = tx
        .send(AgentEvent::SubAgentNotification {
            turn_id: turn_id.clone(),
            completions: completions.to_vec(),
            remaining_active,
        })
        .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_summary(run_id: &str, status: &str) -> CompletionSummary {
        CompletionSummary {
            run_id: run_id.to_string(),
            agent_id: "test-agent".to_string(),
            subagent_type: "explore".to_string(),
            task: "Research something".to_string(),
            status: status.to_string(),
            elapsed_ms: 5000,
            tool_call_count: 3,
            result_preview: Some("Found relevant information.".to_string()),
            error: None,
        }
    }

    #[test]
    fn build_completion_notification_single() {
        let completions = vec![sample_summary("run-1", "completed")];
        let text = build_completion_notification(&completions, 2);
        assert!(text.contains("[Sub-Agent Completed: run-1]"));
        assert!(text.contains("explore"));
        assert!(text.contains("5.0s"));
        assert!(text.contains("3"));
        assert!(text.contains("Remaining active sub-agents: 2"));
        assert!(text.contains("Found relevant information."));
    }

    #[test]
    fn build_completion_notification_all_done() {
        let completions = vec![
            sample_summary("run-1", "completed"),
            sample_summary("run-2", "failed"),
        ];
        let text = build_completion_notification(&completions, 0);
        assert!(text.contains("All sub-agents have completed"));
        assert!(text.contains("[Sub-Agent Completed: run-1]"));
        assert!(text.contains("[Sub-Agent Completed: run-2]"));
    }

    #[test]
    fn notification_as_system_message_has_correct_role() {
        let msg = notification_as_system_message("test notification");
        assert_eq!(msg.role, Role::System);
        assert_eq!(
            msg.content,
            Some(serde_json::Value::String("test notification".to_string()))
        );
        assert_eq!(msg.name, Some("subagent_harness".to_string()));
    }

    #[test]
    fn is_intermediate_ack_detects_no_tool_calls() {
        let summary = TurnSummary {
            turn_id: TurnId::new("test"),
            tool_calls_made: 0,
            iterations: 1,
            usage: None,
            elapsed_ms: 500,
            context_tokens: None,
            context_window: None,
        };
        assert!(is_intermediate_ack(&summary));

        let summary_with_tools = TurnSummary {
            turn_id: TurnId::new("test"),
            tool_calls_made: 2,
            iterations: 1,
            usage: None,
            elapsed_ms: 500,
            context_tokens: None,
            context_window: None,
        };
        assert!(!is_intermediate_ack(&summary_with_tools));
    }

    #[tokio::test]
    async fn wait_for_completions_collects_batch() {
        let (tx, _) = broadcast::channel(64);
        let mut rx = tx.subscribe();

        let s1 = sample_summary("run-1", "completed");
        let s2 = sample_summary("run-2", "completed");

        // Send both before we start waiting (they'll be buffered).
        tx.send(s1.clone()).unwrap();
        tx.send(s2.clone()).unwrap();

        let completions = wait_for_completions(&mut rx, Duration::from_millis(100)).await;
        assert_eq!(completions.len(), 2);
        assert_eq!(completions[0].run_id, "run-1");
        assert_eq!(completions[1].run_id, "run-2");
    }

    #[tokio::test]
    async fn emit_notification_sends_event() {
        let (tx, mut rx) = mpsc::channel(16);
        let turn_id = TurnId::new("turn-1");
        let completions = vec![sample_summary("run-1", "completed")];

        emit_notification_event(&tx, &turn_id, &completions, 0).await;

        let event = rx.recv().await.unwrap();
        match event {
            AgentEvent::SubAgentNotification {
                completions: c,
                remaining_active,
                ..
            } => {
                assert_eq!(c.len(), 1);
                assert_eq!(c[0].run_id, "run-1");
                assert_eq!(remaining_active, 0);
            }
            _ => panic!("unexpected event variant"),
        }
    }
}
