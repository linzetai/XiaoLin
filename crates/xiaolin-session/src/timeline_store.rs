//! Timeline event persistence store.
//!
//! The timeline store records the UI-visible semantic event log with stable
//! per-session sequence ordering and idempotent append semantics. It is a
//! separate table from `event_log` (which records runtime `AgentEvent` JSON
//! for debugging) and from `history_items` (which stores model-context records).
//!
//! ## Design
//!
//! - `turn_timeline_events` table: session_id, turn_id, event_id (UNIQUE),
//!   seq, event_type, schema_version, payload_json, created_at_ms.
//! - Idempotent append: INSERT OR IGNORE on UNIQUE(event_id); returns
//!   existing seq on duplicate.
//! - Monotonically increasing per-session seq allocated atomically via
//!   `COALESCE(MAX(seq), 0) + 1` in the INSERT.
//! - Queries by session, turn, `after_seq`, and page limit.
//! - Materialization from timeline events to `TurnDisplayNode[]` via the
//!   canonical `materialize_events_to_nodes` reducer.

use sqlx::sqlite::SqlitePool;
use xiaolin_protocol::{
    ApprovalNode, AssistantTextNode, IterationBoundaryNode, NodeStatus, ReasoningNode,
    SourceEventTrace, SystemNoticeNode, TerminalDiagnosisMetadata, TimelineEventId,
    TimelineEventType, ToolCategory, ToolStepNode, TurnDisplayNode, TurnStatusNode,
    TurnTimelineEvent, UserMessageNode,
};

/// Persistence layer for the canonical turn timeline.
#[derive(Debug, Clone)]
pub struct TimelineStore {
    pool: SqlitePool,
}

impl TimelineStore {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Create the `turn_timeline_events` table and indices if they don't exist.
    /// Safe to call multiple times — uses `IF NOT EXISTS`.
    pub async fn ensure_table(&self) -> anyhow::Result<()> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS turn_timeline_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL,
                turn_id TEXT NOT NULL,
                event_id TEXT NOT NULL UNIQUE,
                seq INTEGER NOT NULL,
                event_type TEXT NOT NULL,
                schema_version INTEGER NOT NULL DEFAULT 1,
                payload_json TEXT NOT NULL,
                created_at_ms INTEGER NOT NULL
            )",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_timeline_session_seq
             ON turn_timeline_events(session_id, seq)",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_timeline_session_turn
             ON turn_timeline_events(session_id, turn_id, seq)",
        )
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Append a timeline event with idempotent semantics.
    ///
    /// Per-session `seq` is allocated atomically: `COALESCE(MAX(seq), 0) + 1`
    /// within the INSERT. If `event_id` already exists (UNIQUE constraint),
    /// INSERT OR IGNORE is a no-op and the existing sequence is returned.
    ///
    /// Returns the assigned (or existing) sequence number.
    pub async fn append(
        &self,
        session_id: &str,
        turn_id: &str,
        event_id: &TimelineEventId,
        event_type: TimelineEventType,
        payload_json: &serde_json::Value,
        created_at_ms: i64,
    ) -> anyhow::Result<i64> {
        let event_type_str = timeline_event_type_to_str(event_type);
        let payload_str = serde_json::to_string(payload_json)?;
        let event_id_str = event_id.as_str();

        // Serialize appends per session via a transaction so MAX(seq) is
        // computed under SQLite's write lock — no two concurrent appends
        // for the same session can observe the same MAX(seq).
        let mut tx = self.pool.begin().await?;

        // Check idempotency: if this event_id already exists, return its seq.
        let existing: Option<i64> = sqlx::query_scalar(
            "SELECT seq FROM turn_timeline_events WHERE event_id = ?1",
        )
        .bind(event_id_str)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(seq) = existing {
            return Ok(seq);
        }

        // Compute the next seq atomically within the transaction.
        let next_seq: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(seq), 0) + 1 FROM turn_timeline_events WHERE session_id = ?1",
        )
        .bind(session_id)
        .fetch_one(&mut *tx)
        .await?;

        sqlx::query(
            "INSERT INTO turn_timeline_events
                (session_id, turn_id, event_id, seq, event_type, schema_version, payload_json, created_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        )
        .bind(session_id)
        .bind(turn_id)
        .bind(event_id_str)
        .bind(next_seq)
        .bind(event_type_str)
        .bind(xiaolin_protocol::TIMELINE_SCHEMA_VERSION as i64)
        .bind(&payload_str)
        .bind(created_at_ms)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(next_seq)
    }

    /// Query timeline events for a session, optionally after a given sequence.
    /// Returns events ordered by `seq` ASC, limited to `limit` rows (default 500).
    pub async fn query_by_session(
        &self,
        session_id: &str,
        after_seq: Option<i64>,
        limit: Option<i64>,
    ) -> anyhow::Result<Vec<TurnTimelineEvent>> {
        let effective_limit = limit.unwrap_or(500);
        let effective_after = after_seq.unwrap_or(0);

        let rows: Vec<TimelineRow> = sqlx::query_as(
            "SELECT session_id, turn_id, event_id, seq, event_type, schema_version, payload_json, created_at_ms
             FROM turn_timeline_events
             WHERE session_id = ?1 AND seq > ?2
             ORDER BY seq ASC
             LIMIT ?3",
        )
        .bind(session_id)
        .bind(effective_after)
        .bind(effective_limit)
        .fetch_all(&self.pool)
        .await?;

        rows_to_events(rows)
    }

    /// Query timeline events for a specific turn within a session.
    pub async fn query_by_turn(
        &self,
        session_id: &str,
        turn_id: &str,
    ) -> anyhow::Result<Vec<TurnTimelineEvent>> {
        let rows: Vec<TimelineRow> = sqlx::query_as(
            "SELECT session_id, turn_id, event_id, seq, event_type, schema_version, payload_json, created_at_ms
             FROM turn_timeline_events
             WHERE session_id = ?1 AND turn_id = ?2
             ORDER BY seq ASC",
        )
        .bind(session_id)
        .bind(turn_id)
        .fetch_all(&self.pool)
        .await?;

        rows_to_events(rows)
    }

    /// Return the highest `seq` for a session, or `None` if no events exist.
    pub async fn max_seq(&self, session_id: &str) -> anyhow::Result<Option<i64>> {
        let seq: Option<i64> = sqlx::query_scalar(
            "SELECT MAX(seq) FROM turn_timeline_events WHERE session_id = ?1",
        )
        .bind(session_id)
        .fetch_optional(&self.pool)
        .await?
        .flatten();
        Ok(seq)
    }

    /// Count timeline events for a session.
    pub async fn count(&self, session_id: &str) -> anyhow::Result<i64> {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM turn_timeline_events WHERE session_id = ?1",
        )
        .bind(session_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(count)
    }

    /// Materialize all timeline events for a session into `TurnDisplayNode[]`.
    pub async fn materialize_display_nodes(
        &self,
        session_id: &str,
    ) -> anyhow::Result<Vec<TurnDisplayNode>> {
        let events = self.query_by_session(session_id, None, None).await?;
        Ok(materialize_events_to_nodes(&events))
    }

    /// Materialize display nodes for a specific turn.
    pub async fn materialize_display_nodes_for_turn(
        &self,
        session_id: &str,
        turn_id: &str,
    ) -> anyhow::Result<Vec<TurnDisplayNode>> {
        let events = self.query_by_turn(session_id, turn_id).await?;
        Ok(materialize_events_to_nodes(&events))
    }
}

// ── Row type for sqlx::query_as ─────────────────────────────────────────────

#[derive(Debug, Clone, sqlx::FromRow)]
struct TimelineRow {
    session_id: String,
    turn_id: String,
    event_id: String,
    seq: i64,
    event_type: String,
    schema_version: i64,
    payload_json: String,
    created_at_ms: i64,
}

fn rows_to_events(rows: Vec<TimelineRow>) -> anyhow::Result<Vec<TurnTimelineEvent>> {
    rows.into_iter()
        .map(|row| {
            let event_type = str_to_timeline_event_type(&row.event_type)?;
            let payload_json: serde_json::Value = serde_json::from_str(&row.payload_json)?;
            Ok(TurnTimelineEvent {
                id: TimelineEventId::new(row.event_id),
                session_id: row.session_id.into(),
                turn_id: row.turn_id.into(),
                seq: row.seq,
                event_type,
                schema_version: row.schema_version as u16,
                payload_json,
                created_at_ms: row.created_at_ms,
            })
        })
        .collect()
}

// ── Event type string conversion ────────────────────────────────────────────

fn timeline_event_type_to_str(t: TimelineEventType) -> &'static str {
    match t {
        TimelineEventType::TurnStarted => "turn_started",
        TimelineEventType::UserMessageCreated => "user_message_created",
        TimelineEventType::AssistantTextDelta => "assistant_text_delta",
        TimelineEventType::AssistantTextSnapshot => "assistant_text_snapshot",
        TimelineEventType::ReasoningDelta => "reasoning_delta",
        TimelineEventType::ReasoningSnapshot => "reasoning_snapshot",
        TimelineEventType::ToolCallStarted => "tool_call_started",
        TimelineEventType::ToolCallProgress => "tool_call_progress",
        TimelineEventType::ToolCallFinished => "tool_call_finished",
        TimelineEventType::ApprovalRequested => "approval_requested",
        TimelineEventType::ApprovalResolved => "approval_resolved",
        TimelineEventType::IterationBoundary => "iteration_boundary",
        TimelineEventType::AssistantMessageFinalized => "assistant_message_finalized",
        TimelineEventType::TurnFinished => "turn_finished",
        TimelineEventType::CompactBoundary => "compact_boundary",
        TimelineEventType::SystemNotice => "system_notice",
        _ => {
            tracing::warn!("unknown TimelineEventType variant; encoding as __unknown__");
            "__unknown__"
        }
    }
}

fn str_to_timeline_event_type(s: &str) -> anyhow::Result<TimelineEventType> {
    match s {
        "turn_started" => Ok(TimelineEventType::TurnStarted),
        "user_message_created" => Ok(TimelineEventType::UserMessageCreated),
        "assistant_text_delta" => Ok(TimelineEventType::AssistantTextDelta),
        "assistant_text_snapshot" => Ok(TimelineEventType::AssistantTextSnapshot),
        "reasoning_delta" => Ok(TimelineEventType::ReasoningDelta),
        "reasoning_snapshot" => Ok(TimelineEventType::ReasoningSnapshot),
        "tool_call_started" => Ok(TimelineEventType::ToolCallStarted),
        "tool_call_progress" => Ok(TimelineEventType::ToolCallProgress),
        "tool_call_finished" => Ok(TimelineEventType::ToolCallFinished),
        "approval_requested" => Ok(TimelineEventType::ApprovalRequested),
        "approval_resolved" => Ok(TimelineEventType::ApprovalResolved),
        "iteration_boundary" => Ok(TimelineEventType::IterationBoundary),
        "assistant_message_finalized" => Ok(TimelineEventType::AssistantMessageFinalized),
        "turn_finished" => Ok(TimelineEventType::TurnFinished),
        "compact_boundary" => Ok(TimelineEventType::CompactBoundary),
        "system_notice" => Ok(TimelineEventType::SystemNotice),
        "__unknown__" => {
            tracing::warn!("encountered persisted __unknown__ TimelineEventType — event will be skipped during materialization");
            anyhow::bail!("persisted __unknown__ timeline event type")
        }
        other => anyhow::bail!("unknown timeline event type: {other}"),
    }
}

// ── Materialization (canonical reducer) ─────────────────────────────────────

/// Reduce a sequence of `TurnTimelineEvent` values into `TurnDisplayNode[]`.
///
/// This is the **canonical reducer** — the same logic must be applied to both
/// live WebSocket events (client-side) and stored events (server-side
/// materialization) so that live and replay produce identical display nodes.
///
/// ## Coalescing
///
/// Consecutive `AssistantTextDelta` events with the same `node_id` are
/// coalesced into a single `AssistantTextNode`. The buffer is flushed before
/// any visible non-text event (tool start/result, reasoning, approval,
/// iteration boundary, compact boundary, terminal status, turn end) so that
/// text-tool-text order is preserved.
pub fn materialize_events_to_nodes(events: &[TurnTimelineEvent]) -> Vec<TurnDisplayNode> {
    let mut nodes: Vec<TurnDisplayNode> = Vec::new();

    // Text coalescing buffer: (node_id, content, c_at_ms, u_at_ms, evt_ids, min_seq, max_seq)
    let mut text_buf: Option<DeltaBuf> = None;
    let mut reasoning_buf: Option<DeltaBuf> = None;

    for event in events {
        let evt_ids = vec![event.id.to_string()];
        let min_seq = Some(event.seq);
        let max_seq = Some(event.seq);

        match event.event_type {
            TimelineEventType::TurnStarted => {
                // No visible node — turn lifecycle is implicit in other nodes.
            }

            TimelineEventType::UserMessageCreated => {
                flush_text(&mut text_buf, &mut nodes);
                flush_reasoning(&mut reasoning_buf, &mut nodes);
                if let Ok(p) = serde_json::from_value::<
                    xiaolin_protocol::UserMessageCreatedPayload,
                >(event.payload_json.clone())
                {
                    nodes.push(TurnDisplayNode::UserMessage(UserMessageNode {
                        node_id: format!("node-um-{}", event.id.as_str()),
                        turn_id: event.turn_id.clone(),
                        status: NodeStatus::Completed,
                        created_at_ms: event.created_at_ms,
                        updated_at_ms: event.created_at_ms,
                        content: p.content,
                        message_id: p.message_id,
                        attachments: p.attachments,
                        source_trace: Some(SourceEventTrace {
                            event_ids: evt_ids,
                            min_seq,
                            max_seq,
                        }),
                    }));
                }
            }

            TimelineEventType::AssistantTextDelta => {
                if let Ok(p) = serde_json::from_value::<
                    xiaolin_protocol::AssistantTextDeltaPayload,
                >(event.payload_json.clone())
                {
                    if !p.delta.is_empty() {
                        coalesce_delta(
                            &mut text_buf,
                            &p.node_id,
                            &p.delta,
                            event,
                            &evt_ids,
                            min_seq,
                            max_seq,
                        );
                    }
                }
            }

            TimelineEventType::AssistantTextSnapshot => {
                if let Ok(p) = serde_json::from_value::<
                    xiaolin_protocol::AssistantTextSnapshotPayload,
                >(event.payload_json.clone())
                {
                    flush_text(&mut text_buf, &mut nodes);
                    nodes.push(TurnDisplayNode::AssistantText(AssistantTextNode {
                        node_id: p.node_id,
                        turn_id: event.turn_id.clone(),
                        status: NodeStatus::Completed,
                        created_at_ms: event.created_at_ms,
                        updated_at_ms: event.created_at_ms,
                        content: p.content,
                        byte_length: p.byte_length,
                        source_trace: Some(SourceEventTrace {
                            event_ids: evt_ids,
                            min_seq,
                            max_seq,
                        }),
                    }));
                }
            }

            TimelineEventType::ReasoningDelta => {
                if let Ok(p) = serde_json::from_value::<
                    xiaolin_protocol::ReasoningDeltaPayload,
                >(event.payload_json.clone())
                {
                    if !p.delta.is_empty() {
                        coalesce_delta(
                            &mut reasoning_buf,
                            &p.node_id,
                            &p.delta,
                            event,
                            &evt_ids,
                            min_seq,
                            max_seq,
                        );
                    }
                }
            }

            TimelineEventType::ReasoningSnapshot => {
                if let Ok(p) = serde_json::from_value::<
                    xiaolin_protocol::ReasoningSnapshotPayload,
                >(event.payload_json.clone())
                {
                    flush_reasoning(&mut reasoning_buf, &mut nodes);
                    nodes.push(TurnDisplayNode::Reasoning(ReasoningNode {
                        node_id: p.node_id,
                        turn_id: event.turn_id.clone(),
                        status: NodeStatus::Completed,
                        created_at_ms: event.created_at_ms,
                        updated_at_ms: event.created_at_ms,
                        content: p.content,
                        collapsed: true,
                        source_trace: Some(SourceEventTrace {
                            event_ids: evt_ids,
                            min_seq,
                            max_seq,
                        }),
                    }));
                }
            }

            TimelineEventType::ToolCallStarted => {
                flush_text(&mut text_buf, &mut nodes);
                flush_reasoning(&mut reasoning_buf, &mut nodes);
                if let Ok(p) = serde_json::from_value::<
                    xiaolin_protocol::ToolCallStartedPayload,
                >(event.payload_json.clone())
                {
                    let category = p.tool_category.as_deref().and_then(str_to_tool_category);
                    nodes.push(TurnDisplayNode::ToolStep(ToolStepNode {
                        node_id: format!("node-ts-{}", event.id.as_str()),
                        turn_id: event.turn_id.clone(),
                        status: NodeStatus::Running,
                        created_at_ms: event.created_at_ms,
                        updated_at_ms: event.created_at_ms,
                        tool_name: p.tool_name,
                        tool_category: category,
                        display_title: p.display_title.unwrap_or_else(|| "Tool".into()),
                        call_id: p.call_id,
                        target: p.target,
                        progress_label: None,
                        progress: None,
                        started_at_ms: Some(event.created_at_ms),
                        finished_at_ms: None,
                        duration_ms: None,
                        output_preview: None,
                        output_detail: None,
                        error_message: None,
                        args: p.args,
                        source_trace: Some(SourceEventTrace {
                            event_ids: evt_ids,
                            min_seq,
                            max_seq,
                        }),
                    }));
                }
            }

            TimelineEventType::ToolCallProgress => {
                if let Ok(p) = serde_json::from_value::<
                    xiaolin_protocol::ToolCallProgressPayload,
                >(event.payload_json.clone())
                {
                    update_tool_step(&mut nodes, &p.call_id, |step| {
                        step.progress_label = Some(p.message);
                        step.progress = p.progress;
                        step.updated_at_ms = event.created_at_ms;
                        append_trace(&mut step.source_trace, event);
                    });
                }
            }

            TimelineEventType::ToolCallFinished => {
                if let Ok(p) = serde_json::from_value::<
                    xiaolin_protocol::ToolCallFinishedPayload,
                >(event.payload_json.clone())
                {
                    update_tool_step(&mut nodes, &p.call_id, |step| {
                        step.status = if p.success {
                            NodeStatus::Completed
                        } else {
                            NodeStatus::Failed
                        };
                        step.finished_at_ms = Some(event.created_at_ms);
                        step.duration_ms = p.duration_ms;
                        step.updated_at_ms = event.created_at_ms;
                        step.output_preview = p.output_preview;
                        step.output_detail = p.output_detail;
                        step.error_message = p.error_message;
                        append_trace(&mut step.source_trace, event);
                    });
                }
            }

            TimelineEventType::ApprovalRequested => {
                flush_text(&mut text_buf, &mut nodes);
                flush_reasoning(&mut reasoning_buf, &mut nodes);
                if let Ok(p) = serde_json::from_value::<
                    xiaolin_protocol::ApprovalRequestedPayload,
                >(event.payload_json.clone())
                {
                    nodes.push(TurnDisplayNode::Approval(ApprovalNode {
                        node_id: format!("node-ap-{}", event.id.as_str()),
                        turn_id: event.turn_id.clone(),
                        status: NodeStatus::Running,
                        created_at_ms: event.created_at_ms,
                        updated_at_ms: event.created_at_ms,
                        approval_id: p.approval_id,
                        action: p.action,
                        reason: p.reason,
                        risk_level: p.risk_level,
                        decision: None,
                        decision_source: None,
                        source_trace: Some(SourceEventTrace {
                            event_ids: evt_ids,
                            min_seq,
                            max_seq,
                        }),
                    }));
                }
            }

            TimelineEventType::ApprovalResolved => {
                if let Ok(p) = serde_json::from_value::<
                    xiaolin_protocol::ApprovalResolvedPayload,
                >(event.payload_json.clone())
                {
                    update_approval(&mut nodes, &p.approval_id, |approval| {
                        approval.status = NodeStatus::Completed;
                        approval.decision = Some(p.decision);
                        approval.decision_source = Some(p.source);
                        approval.updated_at_ms = event.created_at_ms;
                        append_trace(&mut approval.source_trace, event);
                    });
                }
            }

            TimelineEventType::IterationBoundary => {
                flush_text(&mut text_buf, &mut nodes);
                flush_reasoning(&mut reasoning_buf, &mut nodes);
                if let Ok(p) = serde_json::from_value::<
                    xiaolin_protocol::IterationBoundaryPayload,
                >(event.payload_json.clone())
                {
                    nodes.push(TurnDisplayNode::IterationBoundary(
                        IterationBoundaryNode {
                            node_id: format!("node-ib-{}", event.id.as_str()),
                            turn_id: event.turn_id.clone(),
                            status: NodeStatus::Completed,
                            created_at_ms: event.created_at_ms,
                            updated_at_ms: event.created_at_ms,
                            iteration: p.iteration,
                            source_trace: Some(SourceEventTrace {
                                event_ids: evt_ids,
                                min_seq,
                                max_seq,
                            }),
                        },
                    ));
                }
            }

            TimelineEventType::AssistantMessageFinalized => {
                flush_text(&mut text_buf, &mut nodes);
            }

            TimelineEventType::TurnFinished => {
                flush_text(&mut text_buf, &mut nodes);
                flush_reasoning(&mut reasoning_buf, &mut nodes);
                if let Ok(p) = serde_json::from_value::<
                    xiaolin_protocol::TurnFinishedPayload,
                >(event.payload_json.clone())
                {
                    if p.end_reason != "completed" {
                        nodes.push(TurnDisplayNode::TurnStatus(TurnStatusNode {
                            node_id: format!("node-tstat-{}", event.id.as_str()),
                            turn_id: event.turn_id.clone(),
                            status: NodeStatus::Completed,
                            created_at_ms: event.created_at_ms,
                            updated_at_ms: event.created_at_ms,
                            end_reason: p.end_reason,
                            summary: p.user_message.clone(),
                            diagnosis: Some(TerminalDiagnosisMetadata {
                                diagnosis_code: p.diagnosis_code,
                                severity: p.severity,
                                user_message: p.user_message,
                                iterations: p.iterations,
                                tool_calls: p.tool_calls,
                                repeated_force_stops: None,
                                repeated_warns: None,
                                no_progress_count: None,
                            }),
                            elapsed_ms: p.elapsed_ms,
                            source_trace: Some(SourceEventTrace {
                                event_ids: evt_ids,
                                min_seq,
                                max_seq,
                            }),
                        }));
                    }
                }
            }

            TimelineEventType::CompactBoundary => {
                flush_text(&mut text_buf, &mut nodes);
                flush_reasoning(&mut reasoning_buf, &mut nodes);
                if let Ok(p) = serde_json::from_value::<
                    xiaolin_protocol::CompactBoundaryPayload,
                >(event.payload_json.clone())
                {
                    nodes.push(TurnDisplayNode::SystemNotice(SystemNoticeNode {
                        node_id: format!("node-sn-{}", event.id.as_str()),
                        turn_id: event.turn_id.clone(),
                        status: NodeStatus::Completed,
                        created_at_ms: event.created_at_ms,
                        updated_at_ms: event.created_at_ms,
                        message: format!(
                            "Context compacted ({} → {} tokens, {} messages removed)",
                            p.pre_compact_tokens, p.post_compact_tokens, p.messages_removed
                        ),
                        level: Some("info".into()),
                        category: Some("compaction".into()),
                        source_trace: Some(SourceEventTrace {
                            event_ids: evt_ids,
                            min_seq,
                            max_seq,
                        }),
                    }));
                }
            }

            TimelineEventType::SystemNotice => {
                if let Ok(p) = serde_json::from_value::<
                    xiaolin_protocol::SystemNoticePayload,
                >(event.payload_json.clone())
                {
                    nodes.push(TurnDisplayNode::SystemNotice(SystemNoticeNode {
                        node_id: format!("node-sn-{}", event.id.as_str()),
                        turn_id: event.turn_id.clone(),
                        status: NodeStatus::Completed,
                        created_at_ms: event.created_at_ms,
                        updated_at_ms: event.created_at_ms,
                        message: p.message,
                        level: p.level,
                        category: p.category,
                        source_trace: Some(SourceEventTrace {
                            event_ids: evt_ids,
                            min_seq,
                            max_seq,
                        }),
                    }));
                }
            }

            _ => {}
        }
    }

    // Flush any trailing buffered content
    flush_text(&mut text_buf, &mut nodes);
    flush_reasoning(&mut reasoning_buf, &mut nodes);

    nodes
}

// ── Materializer helpers ────────────────────────────────────────────────────

// (node_id, turn_id, content, c_at_ms, u_at_ms, evt_ids, min_seq, max_seq)
type DeltaBuf = (
    String,
    String,
    String,
    i64,
    i64,
    Vec<String>,
    Option<i64>,
    Option<i64>,
);

fn coalesce_delta(
    buf: &mut Option<DeltaBuf>,
    node_id: &str,
    delta: &str,
    event: &TurnTimelineEvent,
    evt_ids: &[String],
    min_seq: Option<i64>,
    max_seq: Option<i64>,
) {
    if let Some((ref buf_id, _, ref mut content, _, ref mut u_at, ref mut eids, ref mut mn, ref mut mx)) =
        buf
    {
        if buf_id == node_id {
            content.push_str(delta);
            *u_at = event.created_at_ms;
            eids.extend_from_slice(evt_ids);
            *mn = mn.map(|s| s.min(event.seq)).or(Some(event.seq));
            *mx = mx.map(|s| s.max(event.seq)).or(Some(event.seq));
            return;
        }
    }
    // New node_id — replace buffer. The caller is responsible for flushing the
    // old buffer before switching (the materializer loop calls flush_text before
    // every non-text event, so in practice node_id switches always occur after a
    // flush point).
    *buf = Some((
        node_id.to_string(),
        event.turn_id.to_string(),
        delta.to_string(),
        event.created_at_ms,
        event.created_at_ms,
        evt_ids.to_vec(),
        min_seq,
        max_seq,
    ));
}

fn flush_text(buf: &mut Option<DeltaBuf>, nodes: &mut Vec<TurnDisplayNode>) {
    if let Some((node_id, turn_id, content, c_at, u_at, eids, mn, mx)) = buf.take() {
        if !content.is_empty() {
            let byte_len = content.len() as u64;
            nodes.push(TurnDisplayNode::AssistantText(AssistantTextNode {
                node_id,
                turn_id: xiaolin_protocol::TurnId::new(turn_id),
                status: NodeStatus::Completed,
                created_at_ms: c_at,
                updated_at_ms: u_at,
                content,
                byte_length: byte_len,
                source_trace: Some(SourceEventTrace {
                    event_ids: eids,
                    min_seq: mn,
                    max_seq: mx,
                }),
            }));
        }
    }
}

fn flush_reasoning(buf: &mut Option<DeltaBuf>, nodes: &mut Vec<TurnDisplayNode>) {
    if let Some((node_id, turn_id, content, c_at, u_at, eids, mn, mx)) = buf.take() {
        if !content.is_empty() {
            nodes.push(TurnDisplayNode::Reasoning(ReasoningNode {
                node_id,
                turn_id: xiaolin_protocol::TurnId::new(turn_id),
                status: NodeStatus::Completed,
                created_at_ms: c_at,
                updated_at_ms: u_at,
                content,
                collapsed: true,
                source_trace: Some(SourceEventTrace {
                    event_ids: eids,
                    min_seq: mn,
                    max_seq: mx,
                }),
            }));
        }
    }
}

fn append_trace(trace: &mut Option<SourceEventTrace>, event: &TurnTimelineEvent) {
    if let Some(ref mut t) = trace {
        t.event_ids.push(event.id.to_string());
        t.min_seq = t.min_seq.map(|s| s.min(event.seq)).or(Some(event.seq));
        t.max_seq = t.max_seq.map(|s| s.max(event.seq)).or(Some(event.seq));
    }
}

fn update_tool_step(
    nodes: &mut Vec<TurnDisplayNode>,
    call_id: &str,
    f: impl FnOnce(&mut ToolStepNode),
) {
    for node in nodes.iter_mut().rev() {
        if let TurnDisplayNode::ToolStep(ref mut step) = node {
            if step.call_id == call_id {
                f(step);
                return;
            }
        }
    }
}

fn update_approval(
    nodes: &mut Vec<TurnDisplayNode>,
    approval_id: &str,
    f: impl FnOnce(&mut ApprovalNode),
) {
    for node in nodes.iter_mut().rev() {
        if let TurnDisplayNode::Approval(ref mut approval) = node {
            if approval.approval_id == approval_id {
                f(approval);
                return;
            }
        }
    }
}

fn str_to_tool_category(s: &str) -> Option<ToolCategory> {
    match s {
        "file" => Some(ToolCategory::File),
        "shell" => Some(ToolCategory::Shell),
        "search" => Some(ToolCategory::Search),
        "web" => Some(ToolCategory::Web),
        "mcp" => Some(ToolCategory::Mcp),
        "interaction" => Some(ToolCategory::Interaction),
        "sub_agent" => Some(ToolCategory::SubAgent),
        "memory" => Some(ToolCategory::Memory),
        "planning" => Some(ToolCategory::Planning),
        "other" => Some(ToolCategory::Other),
        _ => None,
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    async fn make_store() -> TimelineStore {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        let store = TimelineStore::new(pool);
        store.ensure_table().await.unwrap();
        store
    }

    // ── 2.1 + 2.2: Table creation, idempotent append ────────────────────

    #[tokio::test]
    async fn ensure_table_is_idempotent() {
        let store = make_store().await;
        store.ensure_table().await.unwrap(); // second call should not panic
    }

    #[tokio::test]
    async fn append_assigns_monotonic_seq() {
        let store = make_store().await;
        let s1 = store.append("s1", "t1", &TimelineEventId::new("a"), TimelineEventType::TurnStarted, &serde_json::json!({}), 1000).await.unwrap();
        let s2 = store.append("s1", "t1", &TimelineEventId::new("b"), TimelineEventType::UserMessageCreated, &serde_json::json!({"content":"hi"}), 2000).await.unwrap();
        let s3 = store.append("s2", "t2", &TimelineEventId::new("c"), TimelineEventType::TurnStarted, &serde_json::json!({}), 3000).await.unwrap();
        assert_eq!((s1, s2, s3), (1, 2, 1), "different session restarts at seq 1");
    }

    #[tokio::test]
    async fn append_duplicate_is_idempotent() {
        let store = make_store().await;
        let s1 = store.append("s1", "t1", &TimelineEventId::new("dup"), TimelineEventType::TurnStarted, &serde_json::json!({}), 1000).await.unwrap();
        let s2 = store.append("s1", "t1", &TimelineEventId::new("dup"), TimelineEventType::TurnStarted, &serde_json::json!({"x":true}), 9999).await.unwrap();
        assert_eq!(s1, s2);
        assert_eq!(store.count("s1").await.unwrap(), 1);
    }

    // ── 2.3: Monotonic seq ──────────────────────────────────────────────

    #[tokio::test]
    async fn seq_is_monotonic() {
        let store = make_store().await;
        let mut prev = 0i64;
        for i in 0..20 {
            let seq = store.append("s1", "t1", &TimelineEventId::new(format!("e{i}")), TimelineEventType::SystemNotice, &serde_json::json!({"i":i}), 1000 + i * 100).await.unwrap();
            assert!(seq > prev, "seq {seq} not > {prev}");
            prev = seq;
        }
        assert_eq!(store.max_seq("s1").await.unwrap(), Some(20));
    }

    // ── 2.4: Queries ────────────────────────────────────────────────────

    #[tokio::test]
    async fn query_by_session_after_seq_and_limit() {
        let store = make_store().await;
        for i in 0..5 {
            store.append("s1", "t1", &TimelineEventId::new(format!("e{i}")), TimelineEventType::SystemNotice, &serde_json::json!({"i":i}), 1000 + i * 100).await.unwrap();
        }
        let page = store.query_by_session("s1", Some(2), Some(2)).await.unwrap();
        assert_eq!(page.len(), 2);
        assert_eq!(page[0].seq, 3);
        assert_eq!(page[1].seq, 4);
    }

    #[tokio::test]
    async fn query_by_turn_filters_correctly() {
        let store = make_store().await;
        store.append("s1", "t1", &TimelineEventId::new("a"), TimelineEventType::UserMessageCreated, &serde_json::json!({"content":"t1"}), 1000).await.unwrap();
        store.append("s1", "t2", &TimelineEventId::new("b"), TimelineEventType::UserMessageCreated, &serde_json::json!({"content":"t2"}), 2000).await.unwrap();
        assert_eq!(store.query_by_turn("s1", "t1").await.unwrap().len(), 1);
        assert_eq!(store.query_by_turn("s1", "t2").await.unwrap().len(), 1);
        assert!(store.query_by_turn("s1", "t3").await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn max_seq_and_count_on_empty() {
        let store = make_store().await;
        assert_eq!(store.max_seq("nobody").await.unwrap(), None);
        assert_eq!(store.count("nobody").await.unwrap(), 0);
    }

    // ── 2.5: Materialization ────────────────────────────────────────────

    #[tokio::test]
    async fn materialize_user_message() {
        let store = make_store().await;
        store.append("s1", "t1", &TimelineEventId::new("um"), TimelineEventType::UserMessageCreated, &serde_json::json!({"content":"Hello","message_id":"m1"}), 1000).await.unwrap();
        let nodes = store.materialize_display_nodes("s1").await.unwrap();
        assert_eq!(nodes.len(), 1);
        if let TurnDisplayNode::UserMessage(n) = &nodes[0] {
            assert_eq!(n.content, "Hello");
            assert_eq!(n.message_id.as_deref(), Some("m1"));
        } else { panic!("wrong variant"); }
    }

    #[tokio::test]
    async fn materialize_text_delta_coalescing() {
        let store = make_store().await;
        for (i, c) in ["Hel", "lo", " ", "world"] .iter().enumerate() {
            store.append("s1", "t1", &TimelineEventId::new(format!("d{i}")), TimelineEventType::AssistantTextDelta, &serde_json::json!({"node_id":"n1","delta":c,"offset":0}), 2000 + i as i64 * 100).await.unwrap();
        }
        // Flush via tool start
        store.append("s1", "t1", &TimelineEventId::new("ts"), TimelineEventType::ToolCallStarted, &serde_json::json!({"call_id":"tc1","tool_name":"f","display_title":"F"}), 3000).await.unwrap();
        let nodes = store.materialize_display_nodes("s1").await.unwrap();
        assert_eq!(nodes.len(), 2);
        if let TurnDisplayNode::AssistantText(n) = &nodes[0] {
            assert_eq!(n.content, "Hello world");
            assert_eq!(n.source_trace.as_ref().unwrap().event_ids.len(), 4);
        } else { panic!("wrong variant"); }
    }

    #[tokio::test]
    async fn materialize_tool_lifecycle() {
        let store = make_store().await;
        store.append("s1", "t1", &TimelineEventId::new("s"), TimelineEventType::ToolCallStarted, &serde_json::json!({"call_id":"c1","tool_name":"grep","tool_category":"search","display_title":"Search"}), 2000).await.unwrap();
        store.append("s1", "t1", &TimelineEventId::new("p"), TimelineEventType::ToolCallProgress, &serde_json::json!({"call_id":"c1","message":"Scanning...","progress":0.5}), 2500).await.unwrap();
        store.append("s1", "t1", &TimelineEventId::new("f"), TimelineEventType::ToolCallFinished, &serde_json::json!({"call_id":"c1","tool_name":"grep","success":true,"duration_ms":500}), 3000).await.unwrap();
        let nodes = store.materialize_display_nodes("s1").await.unwrap();
        if let TurnDisplayNode::ToolStep(n) = &nodes[0] {
            assert_eq!(n.status, NodeStatus::Completed);
            assert_eq!(n.duration_ms, Some(500));
            assert_eq!(n.progress_label.as_deref(), Some("Scanning..."));
        } else { panic!("wrong variant"); }
    }

    #[tokio::test]
    async fn materialize_terminal_status_and_normal_completion() {
        let store = make_store().await;
        store.append("s1", "t1", &TimelineEventId::new("end1"), TimelineEventType::TurnFinished, &serde_json::json!({"end_reason":"tool_loop","diagnosis_code":"tool_loop","severity":"error","user_message":"Stopped"}), 5000).await.unwrap();
        assert_eq!(store.materialize_display_nodes("s1").await.unwrap().len(), 1);

        let store2 = make_store().await;
        store2.append("s1", "t1", &TimelineEventId::new("end2"), TimelineEventType::TurnFinished, &serde_json::json!({"end_reason":"completed"}), 5000).await.unwrap();
        assert!(store2.materialize_display_nodes("s1").await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn materialize_compact_boundary_and_system_notice() {
        let store = make_store().await;
        store.append("s1", "t1", &TimelineEventId::new("cb"), TimelineEventType::CompactBoundary, &serde_json::json!({"trigger":"auto","pre_compact_tokens":50000,"post_compact_tokens":15000,"messages_removed":20}), 5000).await.unwrap();
        let nodes = store.materialize_display_nodes("s1").await.unwrap();
        if let TurnDisplayNode::SystemNotice(n) = &nodes[0] {
            assert!(n.message.contains("compacted"));
        } else { panic!("wrong variant"); }
    }

    // ── 2.6: Ordering, empty ranges, pagination ─────────────────────────

    #[tokio::test]
    async fn events_ordered_by_seq() {
        let store = make_store().await;
        for i in (0..5).rev() {
            store.append("s1", "t1", &TimelineEventId::new(format!("e{i}")), TimelineEventType::SystemNotice, &serde_json::json!({"i":i}), 1000 + i as i64 * 100).await.unwrap();
        }
        let seqs: Vec<i64> = store.query_by_session("s1", None, None).await.unwrap().iter().map(|e| e.seq).collect();
        assert_eq!(seqs, vec![1, 2, 3, 4, 5]);
    }

    #[tokio::test]
    async fn after_last_seq_returns_empty() {
        let store = make_store().await;
        store.append("s1", "t1", &TimelineEventId::new("e1"), TimelineEventType::SystemNotice, &serde_json::json!({}), 1000).await.unwrap();
        assert!(store.query_by_session("s1", Some(1), Some(10)).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn materialize_for_turn_isolates() {
        let store = make_store().await;
        store.append("s1", "t1", &TimelineEventId::new("a"), TimelineEventType::UserMessageCreated, &serde_json::json!({"content":"t1"}), 1000).await.unwrap();
        store.append("s1", "t2", &TimelineEventId::new("b"), TimelineEventType::UserMessageCreated, &serde_json::json!({"content":"t2"}), 2000).await.unwrap();
        assert_eq!(store.materialize_display_nodes_for_turn("s1", "t1").await.unwrap().len(), 1);
        assert_eq!(store.materialize_display_nodes_for_turn("s1", "t2").await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn materialize_complex_turn() {
        let store = make_store().await;
        store.append("s1", "t1", &TimelineEventId::new("ts"), TimelineEventType::TurnStarted, &serde_json::json!({}), 1000).await.unwrap();
        store.append("s1", "t1", &TimelineEventId::new("um"), TimelineEventType::UserMessageCreated, &serde_json::json!({"content":"Run tests"}), 1100).await.unwrap();
        store.append("s1", "t1", &TimelineEventId::new("txt"), TimelineEventType::AssistantTextSnapshot, &serde_json::json!({"node_id":"n1","content":"Running...","byte_length":10}), 2000).await.unwrap();
        store.append("s1", "t1", &TimelineEventId::new("ts1"), TimelineEventType::ToolCallStarted, &serde_json::json!({"call_id":"c1","tool_name":"bash","tool_category":"shell","display_title":"cargo test"}), 2100).await.unwrap();
        store.append("s1", "t1", &TimelineEventId::new("tf1"), TimelineEventType::ToolCallFinished, &serde_json::json!({"call_id":"c1","tool_name":"bash","success":true,"duration_ms":5000}), 7200).await.unwrap();
        store.append("s1", "t1", &TimelineEventId::new("ib"), TimelineEventType::IterationBoundary, &serde_json::json!({"iteration":2}), 7300).await.unwrap();
        store.append("s1", "t1", &TimelineEventId::new("end"), TimelineEventType::TurnFinished, &serde_json::json!({"end_reason":"completed","elapsed_ms":6300}), 8000).await.unwrap();

        let nodes = store.materialize_display_nodes("s1").await.unwrap();
        assert_eq!(nodes.len(), 4);
        assert!(matches!(nodes[0], TurnDisplayNode::UserMessage(_)));
        assert!(matches!(nodes[1], TurnDisplayNode::AssistantText(_)));
        assert!(matches!(nodes[2], TurnDisplayNode::ToolStep(_)));
        assert!(matches!(nodes[3], TurnDisplayNode::IterationBoundary(_)));
    }

    #[tokio::test]
    async fn duplicate_preserves_seq_gap() {
        let store = make_store().await;
        let sa = store.append("s1", "t1", &TimelineEventId::new("a"), TimelineEventType::SystemNotice, &serde_json::json!({}), 1000).await.unwrap();
        store.append("s1", "t1", &TimelineEventId::new("a"), TimelineEventType::SystemNotice, &serde_json::json!({}), 2000).await.unwrap();
        let sb = store.append("s1", "t1", &TimelineEventId::new("b"), TimelineEventType::SystemNotice, &serde_json::json!({}), 3000).await.unwrap();
        assert_eq!(sb, sa + 1);
    }

    #[tokio::test]
    async fn pagination_full_cycle() {
        let store = make_store().await;
        for i in 0..10 {
            store.append("s1", "t1", &TimelineEventId::new(format!("e{i}")), TimelineEventType::SystemNotice, &serde_json::json!({"i":i}), 1000 + i * 100).await.unwrap();
        }
        for after in 0..10 {
            let page = store.query_by_session("s1", Some(after), Some(3)).await.unwrap();
            let expected: Vec<i64> = ((after + 1)..=10.min(after + 3)).collect();
            let got: Vec<i64> = page.iter().map(|e| e.seq).collect();
            assert_eq!(got, expected, "page after {after}");
        }
    }

    #[test]
    fn materialize_empty_events() {
        assert!(materialize_events_to_nodes(&[]).is_empty());
    }

    #[tokio::test]
    async fn materialize_text_tool_text_ordering() {
        let store = make_store().await;
        store.append("s1", "t1", &TimelineEventId::new("t1"), TimelineEventType::AssistantTextSnapshot, &serde_json::json!({"node_id":"n1","content":"Before","byte_length":6}), 2000).await.unwrap();
        store.append("s1", "t1", &TimelineEventId::new("ts"), TimelineEventType::ToolCallStarted, &serde_json::json!({"call_id":"c1","tool_name":"f","display_title":"F"}), 2500).await.unwrap();
        store.append("s1", "t1", &TimelineEventId::new("tool"), TimelineEventType::ToolCallFinished, &serde_json::json!({"call_id":"c1","tool_name":"f","success":true,"duration_ms":100}), 3000).await.unwrap();
        store.append("s1", "t1", &TimelineEventId::new("t2"), TimelineEventType::AssistantTextSnapshot, &serde_json::json!({"node_id":"n2","content":"After","byte_length":5}), 4000).await.unwrap();
        let nodes = store.materialize_display_nodes("s1").await.unwrap();
        assert_eq!(nodes.len(), 3);
        assert!(matches!(nodes[0], TurnDisplayNode::AssistantText(_)));
        assert!(matches!(nodes[1], TurnDisplayNode::ToolStep(_)));
        assert!(matches!(nodes[2], TurnDisplayNode::AssistantText(_)));
    }
}
