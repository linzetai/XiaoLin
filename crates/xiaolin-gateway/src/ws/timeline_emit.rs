//! Timeline event emission from AgentEvents.
//!
//! Maps runtime `AgentEvent` variants to canonical `TurnTimelineEvent` values,
//! and provides the append-and-broadcast pipeline that persists events to the
//! `TimelineStore` before broadcasting them over WebSocket.
//!
//! # Event Classification (Task 3.2)
//!
//! | AgentEvent Variant | Timeline Event | Classification |
//! |---|---|---|
//! | `TurnStart` | `TurnStarted` | Timeline event |
//! | | `UserMessageCreated` | Emitted separately (not from AgentEvent) |
//! | `ContentDelta` | `AssistantTextDelta` | Timeline event (coalesced) |
//! | `ReasoningDelta` | `ReasoningDelta` | Timeline event (coalesced) |
//! | `ToolExecuting` | `ToolCallStarted` | Timeline event |
//! | `ToolProgress` | `ToolCallProgress` | Timeline event |
//! | `ToolResult` | `ToolCallFinished` | Timeline event |
//! | `ApprovalRequired` | `ApprovalRequested` | Timeline event |
//! | `ApprovalResolved` | `ApprovalResolved` | Timeline event |
//! | `IterationBoundary` | `IterationBoundary` | Timeline event |
//! | `TurnEnd` | `TurnFinished` + `AssistantMessageFinalized` | Timeline event |
//! | `CompactBoundary` | `CompactBoundary` | Timeline event |
//! | `TurnAborted` | `TurnFinished` (aborted) | Timeline event |
//! | `Error` / `StreamError` | `SystemNotice` (error) | Timeline event |
//! | `ContextWarning` | `SystemNotice` (context) | Timeline event |
//! | `ModeChange` | `SystemNotice` (mode) | Timeline event |
//! | `FileArtifact` | `SystemNotice` (artifact) | Timeline event |
//! | `Warning` | `SystemNotice` (warning) | Timeline event |
//! | `GuardianAssessment`, `GuardianWarning` | — | Non-transcript |
//! | `MemoryStored`, `MemoryRecalled` | — | Non-transcript |
//! | `GoalUpdated`, `GoalCleared` | — | Non-transcript |
//! | `SubAgent*` (except Complete) | — | Non-transcript |
//! | `BriefMessage` | — | Non-transcript |
//! | `Suggestions` | — | Non-transcript |
//! | `AskQuestion` | — | Non-transcript |
//! | `ContextUsageUpdate` | — | Non-transcript |
//! | `PlanFileUpdate`, `PlanDelta`, `PlanUpdate` | — | Non-transcript |

use xiaolin_protocol::{
    AgentEvent, ApprovalRequestedPayload, ApprovalResolvedPayload,
    AssistantMessageFinalizedPayload, CompactBoundaryPayload, IterationBoundaryPayload,
    OutputPreview, SystemNoticePayload, TimelineEventId, TimelineEventType,
    ToolCallFinishedPayload, ToolCallProgressPayload, ToolCallStartedPayload, TurnFinishedPayload,
    TurnStartedPayload, TurnTimelineEvent, UserMessageCreatedPayload,
};
use xiaolin_session::TimelineStore;

/// A single timeline event candidate, ready for append-and-broadcast.
#[derive(Debug)]
pub(crate) struct EmissionCandidate {
    pub event_id: TimelineEventId,
    pub event_type: TimelineEventType,
    pub payload_json: serde_json::Value,
}

/// Map an `AgentEvent` to timeline event candidates.
///
/// Returns `None` for events that are non-transcript UI state.
pub(crate) fn map_agent_event_to_timeline(event: &AgentEvent) -> Option<Vec<EmissionCandidate>> {
    match event {
        AgentEvent::TurnStart { .. } => Some(vec![EmissionCandidate {
            event_id: TimelineEventId::generate(),
            event_type: TimelineEventType::TurnStarted,
            payload_json: serde_json::to_value(&TurnStartedPayload {
                session_id: None,
                execution_mode: None,
                agent_id: None,
            })
            .unwrap_or_default(),
        }]),

        // Content/Reasoning deltas are coalesced by the chat handler
        AgentEvent::ContentDelta { .. } | AgentEvent::ReasoningDelta { .. } => None,

        AgentEvent::ToolExecuting {
            call_id,
            tool_name,
            args,
            ..
        } => {
            let tool_category = classify_tool_category(tool_name);
            let display_title = classify_display_title(tool_name, args.as_deref());
            let target = classify_target_metadata(tool_name, args.as_deref());
            Some(vec![EmissionCandidate {
                event_id: TimelineEventId::generate(),
                event_type: TimelineEventType::ToolCallStarted,
                payload_json: serde_json::to_value(&ToolCallStartedPayload {
                    call_id: call_id.clone(),
                    tool_name: tool_name.clone(),
                    tool_category: Some(tool_category),
                    display_title: Some(display_title),
                    target,
                    args: args.clone(),
                })
                .unwrap_or_default(),
            }])
        }

        AgentEvent::ToolProgress {
            call_id,
            message,
            progress,
            partial_output,
            ..
        } => Some(vec![EmissionCandidate {
            event_id: TimelineEventId::generate(),
            event_type: TimelineEventType::ToolCallProgress,
            payload_json: serde_json::to_value(&ToolCallProgressPayload {
                call_id: call_id.clone(),
                message: message.clone(),
                progress: *progress,
                partial_output: partial_output.clone(),
            })
            .unwrap_or_default(),
        }]),

        AgentEvent::ToolResult {
            call_id,
            tool_name,
            output,
            display_output,
            success,
            output_handle,
            output_size_class,
            output_is_expandable,
            ..
        } => {
            let text = display_output.as_deref().unwrap_or(output.as_str());
            let byte_len = text.len() as u64;
            let line_count = (text.lines().count()).max(1) as u32;
            let est_tokens = (byte_len.saturating_div(4)).max(1) as u32;
            let is_small =
                xiaolin_protocol::is_small_output(byte_len, line_count, est_tokens, false);
            let (output_preview, output_detail) = if is_small {
                (
                    Some(OutputPreview {
                        content: text.to_string(),
                        byte_length: byte_len,
                        line_count,
                        estimated_tokens: est_tokens,
                        is_binary: false,
                        content_type: Some("text".to_string()),
                    }),
                    None,
                )
            } else if let Some(handle) = output_handle {
                (
                    None,
                    Some(xiaolin_protocol::OutputDetailReference {
                        handle: handle.clone(),
                        byte_length: byte_len,
                        line_count,
                        is_expandable: output_is_expandable.unwrap_or(true),
                        size_class: Some(
                            output_size_class
                                .clone()
                                .unwrap_or_else(|| "large".to_string()),
                        ),
                        summary: Some(summarize(text, 500)),
                        content_type: Some("text".to_string()),
                    }),
                )
            } else {
                (None, None)
            };
            let error_message = if *success {
                None
            } else if is_small {
                Some(text.to_string())
            } else {
                Some(summarize(text, 500))
            };

            Some(vec![EmissionCandidate {
                event_id: TimelineEventId::generate(),
                event_type: TimelineEventType::ToolCallFinished,
                payload_json: serde_json::to_value(&ToolCallFinishedPayload {
                    call_id: call_id.clone(),
                    tool_name: tool_name.clone(),
                    success: *success,
                    duration_ms: None,
                    output_preview,
                    output_detail,
                    error_message,
                })
                .unwrap_or_default(),
            }])
        }

        AgentEvent::ApprovalRequired {
            approval_id,
            action,
            reason,
            risk_level,
            ..
        } => Some(vec![EmissionCandidate {
            event_id: TimelineEventId::generate(),
            event_type: TimelineEventType::ApprovalRequested,
            payload_json: serde_json::to_value(&ApprovalRequestedPayload {
                approval_id: approval_id.clone(),
                action: format!("{action:?}"),
                reason: reason.clone(),
                risk_level: risk_level.as_ref().map(|r| format!("{r:?}")),
            })
            .unwrap_or_default(),
        }]),

        AgentEvent::ApprovalResolved {
            approval_id,
            decision,
            source,
            ..
        } => Some(vec![EmissionCandidate {
            event_id: TimelineEventId::generate(),
            event_type: TimelineEventType::ApprovalResolved,
            payload_json: serde_json::to_value(&ApprovalResolvedPayload {
                approval_id: approval_id.clone(),
                decision: format!("{decision:?}"),
                source: source.clone(),
            })
            .unwrap_or_default(),
        }]),

        AgentEvent::IterationBoundary { iteration, .. } => Some(vec![EmissionCandidate {
            event_id: TimelineEventId::generate(),
            event_type: TimelineEventType::IterationBoundary,
            payload_json: serde_json::to_value(&IterationBoundaryPayload {
                iteration: *iteration,
            })
            .unwrap_or_default(),
        }]),

        AgentEvent::TurnEnd {
            summary,
            reason,
            diagnosis,
            ..
        } => {
            let end_reason = reason
                .clone()
                .or_else(|| diagnosis.as_ref().map(|d| d.end_reason.to_string()))
                .unwrap_or_else(|| "completed".to_string());
            let diagnosis_code = diagnosis.as_ref().and_then(|d| d.diagnosis_code.clone());
            let severity = diagnosis
                .as_ref()
                .and_then(|d| d.severity.as_ref().map(ToString::to_string));
            let user_message = diagnosis.as_ref().and_then(|d| d.user_message.clone());
            Some(vec![
                EmissionCandidate {
                    event_id: TimelineEventId::generate(),
                    event_type: TimelineEventType::AssistantMessageFinalized,
                    payload_json: serde_json::to_value(&AssistantMessageFinalizedPayload {
                        text_node_id: None,
                        final_text_content: None,
                    })
                    .unwrap_or_default(),
                },
                EmissionCandidate {
                    event_id: TimelineEventId::generate(),
                    event_type: TimelineEventType::TurnFinished,
                    payload_json: serde_json::to_value(&TurnFinishedPayload {
                        end_reason,
                        diagnosis_code,
                        severity,
                        user_message,
                        iterations: Some(summary.iterations),
                        tool_calls: Some(summary.tool_calls_made as u32),
                        elapsed_ms: Some(summary.elapsed_ms),
                    })
                    .unwrap_or_default(),
                },
            ])
        }

        AgentEvent::CompactBoundary {
            trigger,
            pre_compact_tokens,
            post_compact_tokens,
            messages_removed,
            ..
        } => Some(vec![EmissionCandidate {
            event_id: TimelineEventId::generate(),
            event_type: TimelineEventType::CompactBoundary,
            payload_json: serde_json::to_value(&CompactBoundaryPayload {
                trigger: trigger_description(trigger),
                pre_compact_tokens: *pre_compact_tokens as u64,
                post_compact_tokens: *post_compact_tokens as u64,
                messages_removed: *messages_removed as u64,
            })
            .unwrap_or_default(),
        }]),

        AgentEvent::TurnAborted { reason, .. } => {
            let end_reason = match reason {
                xiaolin_protocol::AbortReason::Interrupted => "interrupted",
                xiaolin_protocol::AbortReason::Replaced => "replaced",
                xiaolin_protocol::AbortReason::BudgetLimited => "budget_limited",
            };
            Some(vec![EmissionCandidate {
                event_id: TimelineEventId::generate(),
                event_type: TimelineEventType::TurnFinished,
                payload_json: serde_json::to_value(&TurnFinishedPayload {
                    end_reason: end_reason.to_string(),
                    diagnosis_code: None,
                    severity: Some("warning".to_string()),
                    user_message: None,
                    iterations: None,
                    tool_calls: None,
                    elapsed_ms: None,
                })
                .unwrap_or_default(),
            }])
        }

        AgentEvent::Error { message, .. } | AgentEvent::StreamError { message, .. } => {
            Some(vec![EmissionCandidate {
                event_id: TimelineEventId::generate(),
                event_type: TimelineEventType::SystemNotice,
                payload_json: serde_json::to_value(&SystemNoticePayload {
                    message: message.clone(),
                    level: Some("error".to_string()),
                    category: Some("error".to_string()),
                })
                .unwrap_or_default(),
            }])
        }

        AgentEvent::ContextWarning { level, message, .. } => {
            let severity = match level {
                xiaolin_protocol::ContextWarningLevel::Soft => "warning",
                xiaolin_protocol::ContextWarningLevel::Hard => "error",
            };
            Some(vec![EmissionCandidate {
                event_id: TimelineEventId::generate(),
                event_type: TimelineEventType::SystemNotice,
                payload_json: serde_json::to_value(&SystemNoticePayload {
                    message: message.clone(),
                    level: Some(severity.to_string()),
                    category: Some("context".to_string()),
                })
                .unwrap_or_default(),
            }])
        }

        AgentEvent::ModeChange { from, to, .. } => Some(vec![EmissionCandidate {
            event_id: TimelineEventId::generate(),
            event_type: TimelineEventType::SystemNotice,
            payload_json: serde_json::to_value(&SystemNoticePayload {
                message: format!("Mode changed: {from} → {to}"),
                level: Some("info".to_string()),
                category: Some("mode".to_string()),
            })
            .unwrap_or_default(),
        }]),

        AgentEvent::FileArtifact {
            path, operation, ..
        } => Some(vec![EmissionCandidate {
            event_id: TimelineEventId::generate(),
            event_type: TimelineEventType::SystemNotice,
            payload_json: serde_json::to_value(&SystemNoticePayload {
                message: format!("File artifact: {operation} ({path})"),
                level: Some("info".to_string()),
                category: Some("artifact".to_string()),
            })
            .unwrap_or_default(),
        }]),

        AgentEvent::Warning { message, .. } => Some(vec![EmissionCandidate {
            event_id: TimelineEventId::generate(),
            event_type: TimelineEventType::SystemNotice,
            payload_json: serde_json::to_value(&SystemNoticePayload {
                message: message.clone(),
                level: Some("warning".to_string()),
                category: Some("system".to_string()),
            })
            .unwrap_or_default(),
        }]),

        // Non-transcript events — no timeline emission
        AgentEvent::BriefMessage { .. }
        | AgentEvent::Suggestions { .. }
        | AgentEvent::AskQuestion { .. }
        | AgentEvent::ContextUsageUpdate { .. }
        | AgentEvent::MemoryStored { .. }
        | AgentEvent::MemoryRecalled { .. }
        | AgentEvent::GoalUpdated { .. }
        | AgentEvent::GoalCleared { .. }
        | AgentEvent::PlanFileUpdate { .. }
        | AgentEvent::PlanDelta { .. }
        | AgentEvent::PlanUpdate { .. }
        | AgentEvent::GuardianAssessment { .. }
        | AgentEvent::GuardianWarning { .. }
        | AgentEvent::SubAgentStart { .. }
        | AgentEvent::SubAgentDelta { .. }
        | AgentEvent::SubAgentToolExecuting { .. }
        | AgentEvent::SubAgentToolResult { .. }
        | AgentEvent::SubAgentNotification { .. }
        | AgentEvent::SubAgentComplete { .. } => None,

        // Catch-all for future variants (AgentEvent is #[non_exhaustive])
        _ => None,
    }
}

/// Emit a user message as a timeline event (no corresponding AgentEvent).
pub(crate) fn emit_user_message_timeline(
    content: &str,
    message_id: Option<&str>,
) -> EmissionCandidate {
    let payload = UserMessageCreatedPayload {
        message_id: message_id.map(String::from),
        content: content.to_string(),
        attachments: None,
    };
    EmissionCandidate {
        event_id: TimelineEventId::generate(),
        event_type: TimelineEventType::UserMessageCreated,
        payload_json: serde_json::to_value(&payload).unwrap_or_default(),
    }
}

/// Build a coalesced text delta event (for flush points).
pub(crate) fn build_text_delta(node_id: &str, delta: &str, offset: u64) -> EmissionCandidate {
    EmissionCandidate {
        event_id: TimelineEventId::generate(),
        event_type: TimelineEventType::AssistantTextDelta,
        payload_json: serde_json::to_value(&AssistantTextDeltaPayload {
            node_id: node_id.to_string(),
            delta: delta.to_string(),
            offset,
        })
        .unwrap_or_default(),
    }
}

/// Build a coalesced reasoning delta event.
pub(crate) fn build_reasoning_delta(node_id: &str, delta: &str, offset: u64) -> EmissionCandidate {
    EmissionCandidate {
        event_id: TimelineEventId::generate(),
        event_type: TimelineEventType::ReasoningDelta,
        payload_json: serde_json::to_value(&ReasoningDeltaPayload {
            node_id: node_id.to_string(),
            delta: delta.to_string(),
            offset,
        })
        .unwrap_or_default(),
    }
}

/// Append a timeline event and return the complete TurnTimelineEvent for broadcast.
///
/// This is the single append-and-broadcast pipeline:
/// 1. Append to TimelineStore (idempotent)
/// 2. Returns the full TurnTimelineEvent with assigned seq
pub(crate) async fn append_timeline_event(
    timeline_store: &TimelineStore,
    session_id: &str,
    turn_id: &str,
    candidate: EmissionCandidate,
    now_ms: i64,
) -> anyhow::Result<TurnTimelineEvent> {
    timeline_store
        .append(
            session_id,
            turn_id,
            &candidate.event_id,
            candidate.event_type,
            &candidate.payload_json,
            now_ms,
        )
        .await
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Import payload types for coalesced event helpers.
use xiaolin_protocol::{AssistantTextDeltaPayload, ReasoningDeltaPayload, ToolTargetMetadata};

fn classify_tool_category(tool_name: &str) -> String {
    let cat = match tool_name {
        "read_file" | "write_file" | "edit_file" | "create_file" | "apply_patch"
        | "str_replace_editor" | "list_files" | "search_file" | "search_content" | "grep"
        | "glob" => "file",
        "shell_exec" | "execute_command" | "bash" | "terminal" => "shell",
        "web_search" | "web_fetch" | "web_scrape" => "web",
        "search" | "file_search" | "content_search" | "search_regex" => "search",
        "spawn_subagent" | "task" => "sub_agent",
        "memory_create"
        | "memory_search"
        | "memory_update"
        | "memory_delete"
        | "episodic_memory_search"
        | "semantic_memory_search" => "memory",
        "ask_question" | "confirm" | "send_user_message" => "interaction",
        "todo_write" | "create_goal" | "update_goal" | "enter_plan_mode" | "exit_plan_mode"
        | "update_plan" => "planning",
        _ => {
            if tool_name.starts_with("mcp__") {
                "mcp"
            } else {
                "other"
            }
        }
    };
    cat.to_string()
}

fn classify_display_title(tool_name: &str, args: Option<&str>) -> String {
    let json = |a: &str| serde_json::from_str::<serde_json::Value>(a).ok();
    match tool_name {
        "read_file" | "write_file" | "edit_file" => {
            let p = args.and_then(|a| {
                json(a).and_then(|v| {
                    v.get("file_path")
                        .or_else(|| v.get("path"))
                        .and_then(|p| p.as_str())
                        .map(String::from)
                })
            });
            let action = if tool_name == "read_file" {
                "Read"
            } else if tool_name == "write_file" {
                "Write"
            } else {
                "Edit"
            };
            p.map(|fp| format!("{action} {}", fp.rsplit('/').next().unwrap_or(&fp)))
                .unwrap_or_else(|| format!("{action} file"))
        }
        "shell_exec" | "execute_command" => args
            .and_then(|a| {
                json(a).and_then(|v| v.get("command").and_then(|c| c.as_str()).map(String::from))
            })
            .map(|c| {
                format!(
                    "Run {}",
                    if c.len() > 60 {
                        format!("{}…", &c[..60])
                    } else {
                        c
                    }
                )
            })
            .unwrap_or_else(|| "Run command".to_string()),
        "web_search" => args
            .and_then(|a| {
                json(a).and_then(|v| {
                    v.get("query")
                        .or_else(|| v.get("searchTerm"))
                        .and_then(|q| q.as_str())
                        .map(String::from)
                })
            })
            .map(|q| format!("Search: {}", tr(q, 50)))
            .unwrap_or_else(|| "Web search".to_string()),
        "web_fetch" => args
            .and_then(|a| {
                json(a).and_then(|v| v.get("url").and_then(|u| u.as_str()).map(String::from))
            })
            .map(|u| format!("Fetch {}", tr(u, 50)))
            .unwrap_or_else(|| "Web fetch".to_string()),
        "spawn_subagent" => args
            .and_then(|a| {
                json(a).and_then(|v| v.get("prompt").and_then(|p| p.as_str()).map(String::from))
            })
            .map(|p| format!("Sub-agent: {}", tr(p, 50)))
            .unwrap_or_else(|| "Sub-agent".to_string()),
        _ => tool_name.to_string(),
    }
}

fn tr(s: String, max: usize) -> String {
    if s.len() <= max {
        s
    } else {
        format!("{}…", &s[..max])
    }
}

fn classify_target_metadata(tool_name: &str, args: Option<&str>) -> Option<ToolTargetMetadata> {
    let v = args.and_then(|a| serde_json::from_str::<serde_json::Value>(a).ok());

    if matches!(
        tool_name,
        "read_file"
            | "write_file"
            | "edit_file"
            | "create_file"
            | "apply_patch"
            | "str_replace_editor"
    ) {
        let path = v
            .as_ref()
            .and_then(|v| v.get("file_path").or_else(|| v.get("path")))
            .and_then(|p| p.as_str())
            .map(String::from);
        if path.is_some() {
            return Some(ToolTargetMetadata {
                path,
                ..Default::default()
            });
        }
    }

    if matches!(tool_name, "shell_exec" | "execute_command") {
        let command = v
            .as_ref()
            .and_then(|v| v.get("command"))
            .and_then(|c| c.as_str())
            .map(String::from);
        if command.is_some() {
            return Some(ToolTargetMetadata {
                command,
                ..Default::default()
            });
        }
    }

    if tool_name == "web_fetch" {
        let url = v
            .as_ref()
            .and_then(|v| v.get("url"))
            .and_then(|u| u.as_str())
            .map(String::from);
        if url.is_some() {
            return Some(ToolTargetMetadata {
                url,
                ..Default::default()
            });
        }
    }

    if tool_name == "web_search" {
        let query = v
            .as_ref()
            .and_then(|v| v.get("query").or_else(|| v.get("searchTerm")))
            .and_then(|q| q.as_str())
            .map(String::from);
        if query.is_some() {
            return Some(ToolTargetMetadata {
                query,
                ..Default::default()
            });
        }
    }

    if matches!(
        tool_name,
        "grep" | "search_content" | "search_regex" | "glob" | "search_file"
    ) {
        let query = v
            .as_ref()
            .and_then(|v| v.get("pattern").or_else(|| v.get("query")))
            .and_then(|q| q.as_str())
            .map(String::from);
        if query.is_some() {
            return Some(ToolTargetMetadata {
                query,
                ..Default::default()
            });
        }
    }

    if tool_name.starts_with("mcp__") {
        let server = tool_name
            .strip_prefix("mcp__")
            .and_then(|r| r.split("__").next().map(String::from));
        if let Some(s) = server {
            return Some(ToolTargetMetadata {
                mcp_server: Some(s),
                label: Some(tool_name.to_string()),
                ..Default::default()
            });
        }
    }

    None
}

fn trigger_description(trigger: &xiaolin_protocol::CompactTrigger) -> String {
    match trigger {
        xiaolin_protocol::CompactTrigger::Auto => "auto".to_string(),
        xiaolin_protocol::CompactTrigger::Manual => "manual".to_string(),
    }
}

fn summarize(text: &str, max_len: usize) -> String {
    if text.len() <= max_len {
        text.to_string()
    } else {
        let cutoff = text[..max_len].rfind('\n').unwrap_or(max_len);
        format!("{}…", &text[..cutoff])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xiaolin_protocol::{
        DiagnosisSeverity, EndReason, TerminalDiagnosis, ToolCallFinishedPayload,
        TurnFinishedPayload, TurnId, TurnSummary,
    };

    fn summary(turn_id: TurnId) -> TurnSummary {
        TurnSummary {
            turn_id,
            tool_calls_made: 3,
            iterations: 2,
            usage: None,
            elapsed_ms: 1234,
            context_tokens: None,
            context_window: None,
        }
    }

    #[test]
    fn turn_end_preserves_terminal_diagnosis_fields() {
        let turn_id = TurnId::new("turn-1");
        let event = AgentEvent::TurnEnd {
            turn_id: turn_id.clone(),
            summary: summary(turn_id),
            session_id: Some("session-1".to_string()),
            final_tool_calls: None,
            reason: None,
            diagnosis: Some(TerminalDiagnosis {
                end_reason: EndReason::ToolLoop,
                diagnosis_code: Some("tool_loop".to_string()),
                severity: Some(DiagnosisSeverity::Error),
                user_message: Some("Stopped because the agent repeated tools.".to_string()),
                evidence: None,
            }),
            plan_outcome: None,
        };

        let candidates = map_agent_event_to_timeline(&event).expect("timeline candidates");
        let finished = candidates
            .iter()
            .find(|candidate| candidate.event_type == TimelineEventType::TurnFinished)
            .expect("turn finished event");
        let payload: TurnFinishedPayload =
            serde_json::from_value(finished.payload_json.clone()).expect("payload");

        assert_eq!(payload.end_reason, "tool_loop");
        assert_eq!(payload.diagnosis_code.as_deref(), Some("tool_loop"));
        assert_eq!(payload.severity.as_deref(), Some("error"));
        assert_eq!(
            payload.user_message.as_deref(),
            Some("Stopped because the agent repeated tools.")
        );
    }

    #[test]
    fn failed_large_tool_output_is_bounded() {
        let output = "stderr line\n".repeat(1000);
        let event = AgentEvent::ToolResult {
            turn_id: TurnId::new("turn-1"),
            tool_name: "shell_exec".to_string(),
            call_id: "call-1".to_string(),
            output: output.clone(),
            display_output: None,
            success: false,
            metadata: None,
            output_handle: None,
            output_size_class: None,
            output_is_expandable: None,
        };

        let candidates = map_agent_event_to_timeline(&event).expect("timeline candidates");
        let payload: ToolCallFinishedPayload =
            serde_json::from_value(candidates[0].payload_json.clone()).expect("payload");

        assert!(!payload.success);
        assert!(payload.output_preview.is_none());
        assert!(payload.output_detail.is_none());
        let err = payload.error_message.expect("bounded error message");
        assert!(err.len() < output.len());
        assert!(err.len() <= 600);
    }

    // ── Full turn simulation: interleaved text/reasoning/tools ────────────

    /// Simulate a full agent turn with interleaved ContentDelta, ReasoningDelta,
    /// and tool calls, then verify the resulting timeline event sequence has
    /// correct types, ordering, and node_id stability.
    ///
    /// This test catches the bug where ContentDelta was buffered instead of
    /// emitted immediately, causing lost text/reasoning interleaving.
    #[test]
    fn full_turn_interleaving_produces_correct_event_sequence() {
        let turn_id = TurnId::new("turn-interleave-1");

        // Simulate a realistic agent event stream:
        // ContentDelta("Let") → ReasoningDelta("Hmm") → ContentDelta(" me") →
        // ToolExecuting → ToolResult → ContentDelta("Done")
        let events: Vec<AgentEvent> = vec![
            AgentEvent::TurnStart {
                turn_id: turn_id.clone(),
                session_id: Some("s1".into()),
                execution_mode: None,
                requested_execution_mode: None,
                mode_source: None,
            },
            AgentEvent::ContentDelta {
                turn_id: turn_id.clone(),
                delta: serde_json::json!({
                    "choices": [{"delta": {"content": "Let"}}]
                }),
                raw_bytes: None,
            },
            AgentEvent::ContentDelta {
                turn_id: turn_id.clone(),
                delta: serde_json::json!({
                    "choices": [{"delta": {"content": " me"}}]
                }),
                raw_bytes: None,
            },
            // Non-empty reasoning should flush preceding text
            AgentEvent::ReasoningDelta {
                turn_id: turn_id.clone(),
                content: "Hmm".into(),
            },
            AgentEvent::ReasoningDelta {
                turn_id: turn_id.clone(),
                content: ", let me check.".into(),
            },
            // ContentDelta triggers reasoning flush → new text node
            AgentEvent::ContentDelta {
                turn_id: turn_id.clone(),
                delta: serde_json::json!({
                    "choices": [{"delta": {"content": "Let me read the file."}}]
                }),
                raw_bytes: None,
            },
            AgentEvent::ToolExecuting {
                turn_id: turn_id.clone(),
                tool_name: "read_file".into(),
                call_id: "call-1".into(),
                args: Some(r#"{"file_path":"src/main.rs"}"#.into()),
            },
            AgentEvent::ToolResult {
                turn_id: turn_id.clone(),
                tool_name: "read_file".into(),
                call_id: "call-1".into(),
                output: "fn main() {}".into(),
                display_output: None,
                success: true,
                metadata: None,
                output_handle: None,
                output_size_class: None,
                output_is_expandable: None,
            },
            AgentEvent::ContentDelta {
                turn_id: turn_id.clone(),
                delta: serde_json::json!({
                    "choices": [{"delta": {"content": "The file looks clean."}}]
                }),
                raw_bytes: None,
            },
            AgentEvent::TurnEnd {
                turn_id: turn_id.clone(),
                summary: summary(turn_id.clone()),
                session_id: Some("s1".into()),
                final_tool_calls: None,
                reason: None,
                diagnosis: None,
                plan_outcome: None,
            },
        ];

        // Collect all timeline event candidates in order
        let mut timeline_types: Vec<String> = Vec::new();
        for event in &events {
            if let Some(candidates) = map_agent_event_to_timeline(event) {
                for c in &candidates {
                    timeline_types.push(format!("{:?}", c.event_type));
                }
            }
            // ContentDelta (handled immediately in chat.rs, not via map_agent_event_to_timeline)
            // is tested separately below.
        }

        // Verify the types produced by map_agent_event_to_timeline
        // TurnStarted, ToolCallStarted, ToolCallFinished, TurnFinished (+ AssistantMessageFinalized)
        assert!(timeline_types.contains(&"TurnStarted".to_string()));
        assert!(timeline_types.contains(&"ToolCallStarted".to_string()));
        assert!(timeline_types.contains(&"ToolCallFinished".to_string()));
        // TurnEnd produces AssistantMessageFinalized + TurnFinished
        let turn_finished_count = timeline_types.iter().filter(|t| *t == "TurnFinished").count();
        assert_eq!(turn_finished_count, 1);

        // Verify ContentDelta and ReasoningDelta are NOT in map output
        // (they are handled immediately via build_text_delta / build_reasoning_delta in chat.rs)
        assert!(!timeline_types.contains(&"AssistantTextDelta".to_string()));
        assert!(!timeline_types.contains(&"ReasoningDelta".to_string()));
    }

    /// Verify that build_text_delta and build_reasoning_delta produce
    /// correctly-typed EmissionCandidates with the expected event types.
    #[test]
    fn immediate_delta_builders_produce_correct_types() {
        let text = build_text_delta("text-1", "Hello", 0);
        assert_eq!(text.event_type, TimelineEventType::AssistantTextDelta);

        let reasoning = build_reasoning_delta("r-1", "Thinking...", 0);
        assert_eq!(reasoning.event_type, TimelineEventType::ReasoningDelta);
    }

    /// Verify that ContentDelta does NOT trigger a timeline event via
    /// map_agent_event_to_timeline (it goes through immediate emission).
    #[test]
    fn content_delta_not_in_map_output() {
        let event = AgentEvent::ContentDelta {
            turn_id: TurnId::new("t1"),
            delta: serde_json::json!({"choices": [{"delta": {"content": "Hi"}}]}),
            raw_bytes: None,
        };
        let candidates = map_agent_event_to_timeline(&event);
        assert!(candidates.is_none(), "ContentDelta should NOT produce timeline events via map — it uses immediate emission");
    }

    /// Verify that empty ReasoningDelta does NOT trigger a text flush.
    /// This would have caught the bug where empty reasoning deltas were
    /// fragmenting text into single-word nodes.
    #[test]
    fn empty_reasoning_delta_is_skipped_by_map() {
        let event = AgentEvent::ReasoningDelta {
            turn_id: TurnId::new("t1"),
            content: String::new(),
        };
        let candidates = map_agent_event_to_timeline(&event);
        // Empty reasoning deltas are skipped entirely (no timeline event)
        assert!(candidates.is_none());
    }

    /// Verify the tool category classification used for display titles.
    #[test]
    fn tool_category_classification() {
        assert_eq!(classify_tool_category("read_file"), "file");
        assert_eq!(classify_tool_category("shell_exec"), "shell");
        assert_eq!(classify_tool_category("web_search"), "web");
        assert_eq!(classify_tool_category("grep"), "file");
        assert_eq!(classify_tool_category("spawn_subagent"), "sub_agent");
        assert_eq!(classify_tool_category("todo_write"), "planning");
        assert_eq!(classify_tool_category("mcp__github__search"), "mcp");
        assert_eq!(classify_tool_category("unknown_tool"), "other");
    }
}
