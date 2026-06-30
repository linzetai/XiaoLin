//! Canonical turn timeline types for UI-visible chat transcript state.
//!
//! This module defines the protocol-level types for the append-only turn
//! timeline: timeline events (the durable event log) and display nodes (the
//! materialized UI contract).  Both live WebSocket rendering and history
//! replay consume these same types through a single reducer contract.

use serde::{Deserialize, Serialize};

#[cfg(feature = "ts")]
use ts_rs::TS;

use crate::id::{SessionId, TurnId};

// ============================================================================
// Timeline event identity
// ============================================================================

/// Globally unique, idempotent identifier for a timeline event.
///
/// If an append is retried with the same id the store returns the existing row
/// instead of duplicating it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[cfg_attr(feature = "ts", ts(as = "String"))]
#[serde(transparent)]
pub struct TimelineEventId(String);

impl TimelineEventId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn generate() -> Self {
        Self(uuid::Uuid::new_v4().to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for TimelineEventId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::ops::Deref for TimelineEventId {
    type Target = str;
    fn deref(&self) -> &str {
        &self.0
    }
}

impl From<String> for TimelineEventId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for TimelineEventId {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

// ============================================================================
// Schema version
// ============================================================================

/// Current schema version for timeline events.
///
/// Increment when the event payload shape changes in a way that would break
/// materialization.  Materializers can use this to decide whether to apply
/// migration logic or reject events.
pub const TIMELINE_SCHEMA_VERSION: u16 = 2;

// ============================================================================
// Reasoning visibility
// ============================================================================

/// Controls whether reasoning content is public or private.
///
/// Private reasoning MUST NOT be persisted, emitted over WebSocket, served
/// via history/reconnect APIs, or rendered by the frontend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ReasoningVisibility {
    /// Visible activity narration — safe for users.
    Public,
    /// Raw provider chain-of-thought — never leaves trusted boundary.
    Private,
}

// ============================================================================
// Assistant text role
// ============================================================================

/// Distinguishes between public activity narration and the final answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum AssistantTextRole {
    /// Public process narration — can be folded into a ProcessInterval.
    Activity,
    /// The final answer — always visible, cuts any active interval.
    Final,
}

// ============================================================================
// Timeline event type enum
// ============================================================================

/// Discriminant for timeline events.
///
/// Each variant maps to a specific lifecycle or content moment in a turn.
/// The payload for each variant is carried in `TurnTimelineEvent.payload_json`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum TimelineEventType {
    /// A new turn has started.
    TurnStarted,
    /// The user's message has been created / accepted.
    UserMessageCreated,
    /// An incremental text delta from the assistant (coalesced before durable
    /// append when possible).
    AssistantTextDelta,
    /// A snapshot of the full assistant text state at a point in time
    /// (emitted after buffered deltas are flushed or on turn end).
    AssistantTextSnapshot,
    /// An incremental reasoning delta from the assistant.
    ReasoningDelta,
    /// A snapshot of the full reasoning state at a point in time.
    ReasoningSnapshot,
    /// A tool call has started.
    ToolCallStarted,
    /// A tool call has progress to report.
    ToolCallProgress,
    /// A tool call has finished (success or failure).
    ToolCallFinished,
    /// An approval has been requested.
    ApprovalRequested,
    /// An approval has been resolved (allowed, denied, etc.).
    ApprovalResolved,
    /// An iteration boundary in the agent loop.
    IterationBoundary,
    /// The assistant message has been finalized for this turn.
    AssistantMessageFinalized,
    /// The turn has finished.
    TurnFinished,
    /// A compaction boundary (auto or manual).
    CompactBoundary,
    /// A system-level notice (not tied to user or assistant action).
    SystemNotice,
}

// ============================================================================
// Timeline event payload structs
// ============================================================================

/// Payload for `TurnStarted` events.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct TurnStartedPayload {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<SessionId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execution_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
}

/// Payload for `UserMessageCreated` events.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct UserMessageCreatedPayload {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
    /// Client-generated id echoed back for optimistic overlay reconciliation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_message_id: Option<String>,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attachments: Option<Vec<String>>,
}

/// Payload for text delta events.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct AssistantTextDeltaPayload {
    /// Target node id so deltas for the same text stream can be coalesced.
    pub node_id: String,
    /// The delta content.
    pub delta: String,
    /// Byte offset from the start of the text stream (for ordering).
    #[serde(default)]
    pub offset: u64,
    /// Whether this is activity narration or final answer text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_role: Option<AssistantTextRole>,
}

/// Payload for text snapshot events.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct AssistantTextSnapshotPayload {
    /// The node id this snapshot represents.
    pub node_id: String,
    /// The full text content at the time of the snapshot.
    pub content: String,
    /// Byte length of the content.
    #[serde(default)]
    pub byte_length: u64,
    /// Whether this is activity narration or final answer text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_role: Option<AssistantTextRole>,
}

/// Payload for reasoning delta events.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct ReasoningDeltaPayload {
    pub node_id: String,
    pub delta: String,
    #[serde(default)]
    pub offset: u64,
    /// Visibility control: private reasoning MUST NOT be persisted or emitted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visibility: Option<ReasoningVisibility>,
}

/// Payload for reasoning snapshot events.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct ReasoningSnapshotPayload {
    pub node_id: String,
    pub content: String,
    /// Visibility control: private reasoning MUST NOT be persisted or emitted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visibility: Option<ReasoningVisibility>,
}

/// Payload for `ToolCallStarted` events.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct ToolCallStartedPayload {
    pub call_id: String,
    pub tool_name: String,
    /// Semantic category for rendering (e.g. "file", "shell", "search").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_category: Option<String>,
    /// Human-readable title for display.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_title: Option<String>,
    /// Target metadata (path, command, URL, query, etc.).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<ToolTargetMetadata>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub args: Option<String>,
}

/// Payload for `ToolCallProgress` events.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct ToolCallProgressPayload {
    pub call_id: String,
    pub message: String,
    /// Progress in [0.0, 1.0] when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub partial_output: Option<String>,
}

/// Payload for `ToolCallFinished` events.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct ToolCallFinishedPayload {
    pub call_id: String,
    pub tool_name: String,
    pub success: bool,
    /// Duration in milliseconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    /// Small output inline preview (when output satisfies small-output policy).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_preview: Option<OutputPreview>,
    /// Large output detail reference (handle into tool output asset store).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_detail: Option<OutputDetailReference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

/// Payload for `ApprovalRequested` events.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct ApprovalRequestedPayload {
    pub approval_id: String,
    pub action: String,
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub risk_level: Option<String>,
}

/// Payload for `ApprovalResolved` events.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct ApprovalResolvedPayload {
    pub approval_id: String,
    pub decision: String,
    pub source: String,
}

/// Payload for `IterationBoundary` events.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct IterationBoundaryPayload {
    pub iteration: u32,
}

/// Payload for `AssistantMessageFinalized` events.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct AssistantMessageFinalizedPayload {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_node_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub final_text_content: Option<String>,
}

/// Payload for `TurnFinished` events.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct TurnFinishedPayload {
    pub end_reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnosis_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub severity: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub iterations: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub elapsed_ms: Option<u64>,
    /// Number of repeated force stops during the turn.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repeated_force_stops: Option<u32>,
    /// Number of repeated warnings during the turn.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repeated_warns: Option<u32>,
    /// Number of consecutive rounds with no progress.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub no_progress_count: Option<u32>,
}

/// Payload for `CompactBoundary` events.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct CompactBoundaryPayload {
    pub trigger: String,
    pub pre_compact_tokens: u64,
    pub post_compact_tokens: u64,
    pub messages_removed: u64,
}

/// Payload for `SystemNotice` events.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct SystemNoticePayload {
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub level: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
}

// ============================================================================
// Tool target metadata
// ============================================================================

/// Target metadata for a tool call — used for compact display titles.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct ToolTargetMetadata {
    /// File path for file operations.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Shell command being executed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// URL for web requests.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Search query string.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    /// MCP server name (for MCP tools).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mcp_server: Option<String>,
    /// Readable label for any other target kind.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

// ============================================================================
// Output preview and detail reference
// ============================================================================

/// Inline output preview for small tool results.
///
/// When tool output satisfies the small-output policy (<= 8,000 UTF-8 bytes,
/// <= 200 lines, <= 2,000 estimated display tokens, not binary), the complete
/// output is included inline so default replay does not need an extra API fetch.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct OutputPreview {
    /// The full text output (when small).
    pub content: String,
    /// Byte length of the output.
    pub byte_length: u64,
    /// Line count.
    pub line_count: u32,
    /// Estimated display tokens.
    pub estimated_tokens: u32,
    /// Whether the content is binary (should always be false here).
    #[serde(default)]
    pub is_binary: bool,
    /// Content type hint: "text", "json", "diff", "command_output", "search_results", "file_listing", "error".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
}

/// Reference to large tool output stored in the tool output asset system.
///
/// The handle string is a session-scoped `ToolOutputHandle` value
/// (e.g. `"out_<sha256_prefix>_<uuid>"`).  The UI detail endpoint resolves
/// this handle against the existing tool output asset store.
///
/// This type carries only the handle string in the protocol layer — the actual
/// `ToolOutputHandle` type lives in `xiaolin-session`.  The protocol layer
/// does not depend on the session crate.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct OutputDetailReference {
    /// The session-scoped tool output handle string.
    pub handle: String,
    /// Total byte size of the output.
    pub byte_length: u64,
    /// Total line count of the output.
    pub line_count: u32,
    /// Whether expansion/full-detail fetching is available.
    #[serde(default = "default_true")]
    pub is_expandable: bool,
    /// Size classification: "medium" | "large".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_class: Option<String>,
    /// A bounded summary of the output content.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Content type hint for rendering.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
}

fn default_true() -> bool {
    true
}

// ============================================================================
// Source event trace metadata
// ============================================================================

/// Trace metadata linking a display node back to its source timeline events.
///
/// Every display node carries optional trace information so consumers can
/// correlate rendered nodes with their originating events.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct SourceEventTrace {
    /// The timeline event ids that contributed to this node.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub event_ids: Vec<String>,
    /// The minimum sequence among contributing events.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_seq: Option<i64>,
    /// The maximum sequence among contributing events.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_seq: Option<i64>,
}

// ============================================================================
// Small-output policy constants
// ============================================================================

/// Maximum UTF-8 byte length for inline (small) tool output.
pub const SMALL_OUTPUT_MAX_BYTES: u64 = 8_000;

/// Maximum line count for inline (small) tool output.
pub const SMALL_OUTPUT_MAX_LINES: u32 = 200;

/// Maximum estimated display tokens for inline (small) tool output.
pub const SMALL_OUTPUT_MAX_TOKENS: u32 = 2_000;

/// Check whether output satisfies the small-output policy.
pub fn is_small_output(
    byte_length: u64,
    line_count: u32,
    estimated_tokens: u32,
    is_binary: bool,
) -> bool {
    !is_binary
        && byte_length <= SMALL_OUTPUT_MAX_BYTES
        && line_count <= SMALL_OUTPUT_MAX_LINES
        && estimated_tokens <= SMALL_OUTPUT_MAX_TOKENS
}

// ============================================================================
// The canonical timeline event
// ============================================================================

/// A single event in the canonical append-only turn timeline.
///
/// This is the durable record.  Events are ordered by monotonically increasing
/// `seq` within a session.  `id` is globally unique and idempotent — appending
/// the same id a second time is a no-op.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct TurnTimelineEvent {
    /// Globally unique, idempotent event identifier.
    pub id: TimelineEventId,
    /// Session this event belongs to.
    pub session_id: SessionId,
    /// Turn this event belongs to.
    pub turn_id: TurnId,
    /// Monotonically increasing per-session sequence number.
    pub seq: i64,
    /// What kind of event this is.
    pub event_type: TimelineEventType,
    /// Schema version at the time this event was written.
    pub schema_version: u16,
    /// The variant-specific payload as a JSON value.
    #[cfg_attr(feature = "ts", ts(type = "Record<string, unknown>"))]
    pub payload_json: serde_json::Value,
    /// Unix timestamp in milliseconds when this event was created.
    pub created_at_ms: i64,
}

// ============================================================================
// Node status
// ============================================================================

/// Status of a display node (used for streaming / lifecycle).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[serde(rename_all = "snake_case")]
pub enum NodeStatus {
    /// The node is being actively produced (streaming).
    Pending,
    /// The node is in progress (e.g. tool running).
    Running,
    /// The node has completed successfully.
    Completed,
    /// The node has failed.
    Failed,
    /// The node was cancelled before completion.
    Cancelled,
}

impl Default for NodeStatus {
    fn default() -> Self {
        Self::Pending
    }
}

// ============================================================================
// Tool category
// ============================================================================

/// Semantic category for tool calls — used for icon selection and grouping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[serde(rename_all = "snake_case")]
pub enum ToolCategory {
    /// File system operations (read, write, edit, glob, ls).
    File,
    /// Shell command execution.
    Shell,
    /// Search operations (grep, find, web search).
    Search,
    /// Web browsing / fetching.
    Web,
    /// MCP tool server calls.
    Mcp,
    /// Approval or user interaction tools.
    Interaction,
    /// Sub-agent spawning or management.
    SubAgent,
    /// Memory operations.
    Memory,
    /// Planning or task management tools.
    Planning,
    /// Anything not covered by the categories above.
    Other,
}

// ============================================================================
// Display node types
// ============================================================================

/// A display-ready node in the chat transcript.
///
/// This is the materialized UI contract.  Both live streaming and history
/// replay produce `TurnDisplayNode` values from the same reducer semantics.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TurnDisplayNode {
    /// A user message.
    UserMessage(UserMessageNode),
    /// Assistant text content.
    AssistantText(AssistantTextNode),
    /// Assistant reasoning content.
    Reasoning(ReasoningNode),
    /// A single tool call step.
    ToolStep(ToolStepNode),
    /// A group of adjacent repetitive tool steps.
    ToolGroup(ToolGroupNode),
    /// An approval request / resolution.
    Approval(ApprovalNode),
    /// An iteration boundary marker.
    IterationBoundary(IterationBoundaryNode),
    /// A terminal turn status indicator.
    TurnStatus(TurnStatusNode),
    /// A system-level notice.
    SystemNotice(SystemNoticeNode),
}

/// A user message node.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct UserMessageNode {
    /// Stable node identifier.
    pub node_id: String,
    pub turn_id: TurnId,
    pub status: NodeStatus,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    /// The user's message content.
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attachments: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_trace: Option<SourceEventTrace>,
}

/// An assistant text node.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct AssistantTextNode {
    pub node_id: String,
    pub turn_id: TurnId,
    pub status: NodeStatus,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    /// The text content (may be partial during streaming).
    pub content: String,
    /// Total byte length (used for display sizing).
    #[serde(default)]
    pub byte_length: u64,
    /// Whether this is activity narration or final answer text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_role: Option<AssistantTextRole>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_trace: Option<SourceEventTrace>,
}

/// A reasoning content node.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct ReasoningNode {
    pub node_id: String,
    pub turn_id: TurnId,
    pub status: NodeStatus,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub content: String,
    /// Whether reasoning is collapsed by default after completion.
    #[serde(default)]
    pub collapsed: bool,
    /// Visibility control: private reasoning must not be rendered.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visibility: Option<ReasoningVisibility>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_trace: Option<SourceEventTrace>,
}

/// A single tool call step node.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct ToolStepNode {
    pub node_id: String,
    pub turn_id: TurnId,
    pub status: NodeStatus,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    /// The tool name.
    pub tool_name: String,
    /// Semantic category for icon/grouping.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_category: Option<ToolCategory>,
    /// Human-readable display title.
    pub display_title: String,
    /// Tool call id for correlation.
    pub call_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<ToolTargetMetadata>,
    /// Progress label when running.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress_label: Option<String>,
    /// Progress in [0.0, 1.0].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<f64>,
    /// Started timestamp (ms).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at_ms: Option<i64>,
    /// Finished timestamp (ms).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_at_ms: Option<i64>,
    /// Duration in ms.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    /// Inline output preview (small output) or summary.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_preview: Option<OutputPreview>,
    /// Large output detail reference for lazy expansion.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_detail: Option<OutputDetailReference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    /// Tool arguments (for lazy detail expansion).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub args: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_trace: Option<SourceEventTrace>,
}

/// A group of adjacent repetitive tool steps.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct ToolGroupNode {
    pub node_id: String,
    pub turn_id: TurnId,
    pub status: NodeStatus,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    /// Summary label for the group.
    pub group_label: String,
    /// Number of steps in the group.
    pub step_count: u32,
    /// Individual step nodes (loaded lazily when expanded).
    pub steps: Vec<ToolStepNode>,
    /// Whether the group is collapsed by default.
    #[serde(default = "default_true")]
    pub collapsed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_trace: Option<SourceEventTrace>,
}

/// An approval request / resolution node.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct ApprovalNode {
    pub node_id: String,
    pub turn_id: TurnId,
    pub status: NodeStatus,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub approval_id: String,
    pub action: String,
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub risk_level: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decision: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decision_source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_trace: Option<SourceEventTrace>,
}

/// An iteration boundary node.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct IterationBoundaryNode {
    pub node_id: String,
    pub turn_id: TurnId,
    pub status: NodeStatus,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    /// The iteration number.
    pub iteration: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_trace: Option<SourceEventTrace>,
}

/// Terminal diagnosis metadata for a `TurnStatusNode`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct TerminalDiagnosisMetadata {
    /// Machine-readable diagnosis code (e.g. "tool_loop", "budget_exceeded").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnosis_code: Option<String>,
    /// Severity: "info", "warning", or "error".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub severity: Option<String>,
    /// User-visible message.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_message: Option<String>,
    /// Number of iterations the turn ran.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub iterations: Option<u32>,
    /// Number of tool calls made.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<u32>,
    /// Number of repeated force stops.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repeated_force_stops: Option<u32>,
    /// Number of repeated warnings.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repeated_warns: Option<u32>,
    /// Number of consecutive rounds with no progress.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub no_progress_count: Option<u32>,
}

/// A terminal turn status indicator node.
///
/// Rendered at the end of a turn for non-successful or diagnostically
/// important turn endings.  A partial assistant text node MAY remain visible
/// before this node, but the transcript must not make tool-loop, cancellation,
/// or runtime failure look like a normally completed response.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct TurnStatusNode {
    pub node_id: String,
    pub turn_id: TurnId,
    pub status: NodeStatus,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    /// Machine-readable end reason.
    pub end_reason: String,
    /// Human-readable status summary.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Terminal diagnosis metadata for abnormal endings.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnosis: Option<TerminalDiagnosisMetadata>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub elapsed_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_trace: Option<SourceEventTrace>,
}

/// A system-level notice node (not tied to user or assistant action).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct SystemNoticeNode {
    pub node_id: String,
    pub turn_id: TurnId,
    pub status: NodeStatus,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub level: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_trace: Option<SourceEventTrace>,
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_session_id() -> SessionId {
        SessionId::new("sess-test-1")
    }

    fn sample_turn_id() -> TurnId {
        TurnId::new("turn-test-1")
    }

    // ── TimelineEventId ────────────────────────────────────────────────

    #[test]
    fn timeline_event_id_roundtrip() {
        let id = TimelineEventId::new("evt-001");
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, "\"evt-001\"");
        let back: TimelineEventId = serde_json::from_str(&json).unwrap();
        assert_eq!(back, id);
    }

    #[test]
    fn timeline_event_id_generate_unique() {
        let a = TimelineEventId::generate();
        let b = TimelineEventId::generate();
        assert_ne!(a, b);
    }

    // ── TimelineEventType ──────────────────────────────────────────────

    #[test]
    fn timeline_event_type_serde_roundtrip() {
        let cases = vec![
            (TimelineEventType::TurnStarted, "turn_started"),
            (
                TimelineEventType::UserMessageCreated,
                "user_message_created",
            ),
            (
                TimelineEventType::AssistantTextDelta,
                "assistant_text_delta",
            ),
            (
                TimelineEventType::AssistantTextSnapshot,
                "assistant_text_snapshot",
            ),
            (TimelineEventType::ReasoningDelta, "reasoning_delta"),
            (TimelineEventType::ReasoningSnapshot, "reasoning_snapshot"),
            (TimelineEventType::ToolCallStarted, "tool_call_started"),
            (TimelineEventType::ToolCallProgress, "tool_call_progress"),
            (TimelineEventType::ToolCallFinished, "tool_call_finished"),
            (TimelineEventType::ApprovalRequested, "approval_requested"),
            (TimelineEventType::ApprovalResolved, "approval_resolved"),
            (TimelineEventType::IterationBoundary, "iteration_boundary"),
            (
                TimelineEventType::AssistantMessageFinalized,
                "assistant_message_finalized",
            ),
            (TimelineEventType::TurnFinished, "turn_finished"),
            (TimelineEventType::CompactBoundary, "compact_boundary"),
            (TimelineEventType::SystemNotice, "system_notice"),
        ];

        for (variant, expected_str) in cases {
            let json = serde_json::to_string(&variant).unwrap();
            assert_eq!(json, format!("\"{expected_str}\""));
            let back: TimelineEventType = serde_json::from_str(&json).unwrap();
            assert_eq!(back, variant);
        }
    }

    // ── TurnTimelineEvent ──────────────────────────────────────────────

    #[test]
    fn turn_timeline_event_roundtrip() {
        let event = TurnTimelineEvent {
            id: TimelineEventId::new("evt-001"),
            session_id: sample_session_id(),
            turn_id: sample_turn_id(),
            seq: 1,
            event_type: TimelineEventType::UserMessageCreated,
            schema_version: TIMELINE_SCHEMA_VERSION,
            payload_json: serde_json::json!({
                "content": "Hello, world!",
                "message_id": "msg-1",
                "attachments": null
            }),
            created_at_ms: 1719000000000,
        };

        let json = serde_json::to_string(&event).unwrap();
        let back: TurnTimelineEvent = serde_json::from_str(&json).unwrap();

        assert_eq!(back.id, event.id);
        assert_eq!(back.session_id, event.session_id);
        assert_eq!(back.turn_id, event.turn_id);
        assert_eq!(back.seq, 1);
        assert_eq!(back.event_type, TimelineEventType::UserMessageCreated);
        assert_eq!(back.schema_version, TIMELINE_SCHEMA_VERSION);
        assert_eq!(back.created_at_ms, 1719000000000);
    }

    #[test]
    fn turn_timeline_event_json_tag() {
        let event = TurnTimelineEvent {
            id: TimelineEventId::new("evt-001"),
            session_id: sample_session_id(),
            turn_id: sample_turn_id(),
            seq: 1,
            event_type: TimelineEventType::TurnStarted,
            schema_version: TIMELINE_SCHEMA_VERSION,
            payload_json: serde_json::json!({"session_id": "sess-1"}),
            created_at_ms: 0,
        };
        let val = serde_json::to_value(&event).unwrap();
        assert_eq!(val["event_type"], "turn_started");
        assert_eq!(val["seq"], 1);
        assert_eq!(val["schema_version"], 2);
    }

    // ── Payload structs ────────────────────────────────────────────────

    #[test]
    fn turn_started_payload_roundtrip() {
        let payload = TurnStartedPayload {
            session_id: Some(SessionId::new("sess-1")),
            execution_mode: Some("agent".into()),
            agent_id: Some("main".into()),
        };
        let json = serde_json::to_value(&payload).unwrap();
        assert_eq!(json["execution_mode"], "agent");

        let back: TurnStartedPayload = serde_json::from_value(json).unwrap();
        assert_eq!(back.execution_mode.unwrap(), "agent");
    }

    #[test]
    fn user_message_created_payload_roundtrip() {
        let payload = UserMessageCreatedPayload {
            message_id: Some("msg-1".into()),
            client_message_id: Some("client-msg-1".into()),
            content: "Hello".into(),
            attachments: Some(vec!["file.txt".into()]),
        };
        let json = serde_json::to_string(&payload).unwrap();
        let back: UserMessageCreatedPayload = serde_json::from_str(&json).unwrap();
        assert_eq!(back.content, "Hello");
        assert_eq!(back.message_id.unwrap(), "msg-1");
        assert_eq!(back.client_message_id.unwrap(), "client-msg-1");
    }

    #[test]
    fn assistant_text_delta_payload_roundtrip() {
        let payload = AssistantTextDeltaPayload {
            node_id: "node-1".into(),
            delta: "hello ".into(),
            offset: 0,
            text_role: None,
        };
        let json = serde_json::to_string(&payload).unwrap();
        let back: AssistantTextDeltaPayload = serde_json::from_str(&json).unwrap();
        assert_eq!(back.node_id, "node-1");
        assert_eq!(back.delta, "hello ");
    }

    #[test]
    fn tool_call_started_payload_roundtrip() {
        let payload = ToolCallStartedPayload {
            call_id: "tc-1".into(),
            tool_name: "read_file".into(),
            tool_category: Some("file".into()),
            display_title: Some("Read src/main.rs".into()),
            target: Some(ToolTargetMetadata {
                path: Some("src/main.rs".into()),
                ..Default::default()
            }),
            args: Some(r#"{"path":"src/main.rs"}"#.into()),
        };
        let json = serde_json::to_value(&payload).unwrap();
        assert_eq!(json["call_id"], "tc-1");
        assert_eq!(json["target"]["path"], "src/main.rs");

        let back: ToolCallStartedPayload = serde_json::from_value(json).unwrap();
        assert_eq!(back.call_id, "tc-1");
        assert!(back.target.is_some());
    }

    #[test]
    fn tool_call_finished_payload_with_output_preview_roundtrip() {
        let payload = ToolCallFinishedPayload {
            call_id: "tc-1".into(),
            tool_name: "read_file".into(),
            success: true,
            duration_ms: Some(150),
            output_preview: Some(OutputPreview {
                content: "file contents here".into(),
                byte_length: 18,
                line_count: 1,
                estimated_tokens: 5,
                is_binary: false,
                content_type: Some("text".into()),
            }),
            output_detail: None,
            error_message: None,
        };
        let json = serde_json::to_string(&payload).unwrap();
        let back: ToolCallFinishedPayload = serde_json::from_str(&json).unwrap();
        assert!(back.success);
        assert_eq!(back.output_preview.unwrap().content, "file contents here");
    }

    #[test]
    fn tool_call_finished_payload_with_output_detail_roundtrip() {
        let payload = ToolCallFinishedPayload {
            call_id: "tc-2".into(),
            tool_name: "bash".into(),
            success: true,
            duration_ms: Some(5000),
            output_preview: None,
            output_detail: Some(OutputDetailReference {
                handle: "out_abc12345_def56789".into(),
                byte_length: 50_000,
                line_count: 1_200,
                is_expandable: true,
                size_class: Some("large".into()),
                summary: Some("Build output (50,000 bytes, 1,200 lines)".into()),
                content_type: Some("command_output".into()),
            }),
            error_message: None,
        };
        let json = serde_json::to_string(&payload).unwrap();
        let back: ToolCallFinishedPayload = serde_json::from_str(&json).unwrap();
        assert!(back.success);
        let detail = back.output_detail.unwrap();
        assert_eq!(detail.handle, "out_abc12345_def56789");
        assert_eq!(detail.byte_length, 50_000);
    }

    #[test]
    fn approval_payloads_roundtrip() {
        let req = ApprovalRequestedPayload {
            approval_id: "apr-1".into(),
            action: "execute_command".into(),
            reason: "Potentially destructive operation".into(),
            risk_level: Some("high".into()),
        };
        let json = serde_json::to_string(&req).unwrap();
        let back: ApprovalRequestedPayload = serde_json::from_str(&json).unwrap();
        assert_eq!(back.approval_id, "apr-1");
        assert_eq!(back.risk_level.unwrap(), "high");

        let res = ApprovalResolvedPayload {
            approval_id: "apr-1".into(),
            decision: "allow_once".into(),
            source: "user".into(),
        };
        let json = serde_json::to_string(&res).unwrap();
        let back: ApprovalResolvedPayload = serde_json::from_str(&json).unwrap();
        assert_eq!(back.decision, "allow_once");
    }

    #[test]
    fn turn_finished_payload_roundtrip() {
        let payload = TurnFinishedPayload {
            end_reason: "tool_loop".into(),
            diagnosis_code: Some("tool_loop".into()),
            severity: Some("error".into()),
            user_message: Some("Turn stopped by tool loop protection.".into()),
            iterations: Some(12),
            tool_calls: Some(45),
            elapsed_ms: Some(35000),
            repeated_force_stops: Some(2),
            repeated_warns: Some(1),
            no_progress_count: None,
        };
        let json = serde_json::to_string(&payload).unwrap();
        let back: TurnFinishedPayload = serde_json::from_str(&json).unwrap();
        assert_eq!(back.end_reason, "tool_loop");
        assert_eq!(back.iterations, Some(12));
        assert_eq!(back.elapsed_ms, Some(35000));
    }

    #[test]
    fn compact_boundary_payload_roundtrip() {
        let payload = CompactBoundaryPayload {
            trigger: "auto".into(),
            pre_compact_tokens: 50_000,
            post_compact_tokens: 15_000,
            messages_removed: 20,
        };
        let json = serde_json::to_string(&payload).unwrap();
        let back: CompactBoundaryPayload = serde_json::from_str(&json).unwrap();
        assert_eq!(back.trigger, "auto");
        assert_eq!(back.pre_compact_tokens, 50_000);
    }

    #[test]
    fn system_notice_payload_roundtrip() {
        let payload = SystemNoticePayload {
            message: "Session context was compacted.".into(),
            level: Some("info".into()),
            category: Some("compaction".into()),
        };
        let json = serde_json::to_string(&payload).unwrap();
        let back: SystemNoticePayload = serde_json::from_str(&json).unwrap();
        assert_eq!(back.message, "Session context was compacted.");
    }

    // ── Display nodes ──────────────────────────────────────────────────

    #[test]
    fn turn_display_node_tagged_serde() {
        let node = TurnDisplayNode::UserMessage(UserMessageNode {
            node_id: "node-um-1".into(),
            turn_id: sample_turn_id(),
            status: NodeStatus::Completed,
            created_at_ms: 1000,
            updated_at_ms: 1000,
            content: "Hello".into(),
            message_id: Some("msg-1".into()),
            attachments: None,
            source_trace: None,
        });
        let val = serde_json::to_value(&node).unwrap();
        assert_eq!(val["kind"], "user_message");
        assert_eq!(val["content"], "Hello");

        let back: TurnDisplayNode = serde_json::from_value(val).unwrap();
        match back {
            TurnDisplayNode::UserMessage(n) => assert_eq!(n.content, "Hello"),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn assistant_text_node_roundtrip() {
        let node = AssistantTextNode {
            node_id: "node-at-1".into(),
            turn_id: sample_turn_id(),
            status: NodeStatus::Completed,
            created_at_ms: 1000,
            updated_at_ms: 2000,
            content: "Here is the result.".into(),
            byte_length: 19,
            text_role: Some(AssistantTextRole::Final),
            source_trace: Some(SourceEventTrace {
                event_ids: vec!["evt-1".into(), "evt-2".into()],
                min_seq: Some(3),
                max_seq: Some(5),
            }),
        };
        let json = serde_json::to_string(&node).unwrap();
        let back: AssistantTextNode = serde_json::from_str(&json).unwrap();
        assert_eq!(back.content, "Here is the result.");
        assert_eq!(back.status, NodeStatus::Completed);
        let trace = back.source_trace.unwrap();
        assert_eq!(trace.event_ids.len(), 2);
        assert_eq!(trace.min_seq, Some(3));
    }

    #[test]
    fn reasoning_node_roundtrip() {
        let node = ReasoningNode {
            node_id: "node-r-1".into(),
            turn_id: sample_turn_id(),
            status: NodeStatus::Completed,
            created_at_ms: 1000,
            updated_at_ms: 1500,
            content: "Let me think about this...".into(),
            collapsed: true,
            visibility: Some(ReasoningVisibility::Public),
            source_trace: None,
        };
        let json = serde_json::to_string(&node).unwrap();
        let back: ReasoningNode = serde_json::from_str(&json).unwrap();
        assert_eq!(back.content, "Let me think about this...");
        assert!(back.collapsed);
    }

    #[test]
    fn tool_step_node_roundtrip() {
        let node = ToolStepNode {
            node_id: "node-ts-1".into(),
            turn_id: sample_turn_id(),
            status: NodeStatus::Completed,
            created_at_ms: 2000,
            updated_at_ms: 2500,
            tool_name: "read_file".into(),
            tool_category: Some(ToolCategory::File),
            display_title: "Read src/main.rs".into(),
            call_id: "tc-1".into(),
            target: Some(ToolTargetMetadata {
                path: Some("src/main.rs".into()),
                ..Default::default()
            }),
            progress_label: None,
            progress: None,
            started_at_ms: Some(2000),
            finished_at_ms: Some(2500),
            duration_ms: Some(500),
            output_preview: Some(OutputPreview {
                content: "fn main() {}".into(),
                byte_length: 12,
                line_count: 1,
                estimated_tokens: 4,
                is_binary: false,
                content_type: Some("text".into()),
            }),
            output_detail: None,
            error_message: None,
            args: Some(r#"{"path":"src/main.rs"}"#.into()),
            source_trace: None,
        };
        let json = serde_json::to_string(&node).unwrap();
        let back: ToolStepNode = serde_json::from_str(&json).unwrap();
        assert_eq!(back.tool_name, "read_file");
        assert_eq!(back.tool_category, Some(ToolCategory::File));
        assert_eq!(back.display_title, "Read src/main.rs");
        assert_eq!(back.duration_ms, Some(500));
        assert!(back.output_preview.is_some());
    }

    #[test]
    fn tool_group_node_roundtrip() {
        let node = ToolGroupNode {
            node_id: "node-tg-1".into(),
            turn_id: sample_turn_id(),
            status: NodeStatus::Completed,
            created_at_ms: 3000,
            updated_at_ms: 4000,
            group_label: "3 file reads".into(),
            step_count: 3,
            steps: vec![],
            collapsed: true,
            source_trace: None,
        };
        let json = serde_json::to_string(&node).unwrap();
        let back: ToolGroupNode = serde_json::from_str(&json).unwrap();
        assert_eq!(back.group_label, "3 file reads");
        assert_eq!(back.step_count, 3);
        assert!(back.collapsed);
    }

    #[test]
    fn approval_node_roundtrip() {
        let node = ApprovalNode {
            node_id: "node-ap-1".into(),
            turn_id: sample_turn_id(),
            status: NodeStatus::Completed,
            created_at_ms: 5000,
            updated_at_ms: 8000,
            approval_id: "apr-1".into(),
            action: "execute_command".into(),
            reason: "Potentially destructive".into(),
            risk_level: Some("high".into()),
            decision: Some("allow_once".into()),
            decision_source: Some("user".into()),
            source_trace: None,
        };
        let json = serde_json::to_string(&node).unwrap();
        let back: ApprovalNode = serde_json::from_str(&json).unwrap();
        assert_eq!(back.approval_id, "apr-1");
        assert_eq!(back.decision, Some("allow_once".into()));
    }

    #[test]
    fn iteration_boundary_node_roundtrip() {
        let node = IterationBoundaryNode {
            node_id: "node-ib-1".into(),
            turn_id: sample_turn_id(),
            status: NodeStatus::Completed,
            created_at_ms: 3000,
            updated_at_ms: 3000,
            iteration: 3,
            source_trace: None,
        };
        let json = serde_json::to_string(&node).unwrap();
        let back: IterationBoundaryNode = serde_json::from_str(&json).unwrap();
        assert_eq!(back.iteration, 3);
    }

    #[test]
    fn turn_status_node_roundtrip() {
        let node = TurnStatusNode {
            node_id: "node-ts-1".into(),
            turn_id: sample_turn_id(),
            status: NodeStatus::Completed,
            created_at_ms: 10_000,
            updated_at_ms: 10_000,
            end_reason: "tool_loop".into(),
            summary: Some("Turn stopped: tool loop detected".into()),
            diagnosis: Some(TerminalDiagnosisMetadata {
                diagnosis_code: Some("tool_loop".into()),
                severity: Some("error".into()),
                user_message: Some("Turn stopped by tool loop protection.".into()),
                iterations: Some(12),
                tool_calls: Some(45),
                repeated_force_stops: Some(2),
                repeated_warns: Some(1),
                no_progress_count: None,
            }),
            elapsed_ms: Some(35_000),
            source_trace: None,
        };
        let json = serde_json::to_string(&node).unwrap();
        let back: TurnStatusNode = serde_json::from_str(&json).unwrap();
        assert_eq!(back.end_reason, "tool_loop");
        let diag = back.diagnosis.unwrap();
        assert_eq!(diag.diagnosis_code.unwrap(), "tool_loop");
        assert_eq!(diag.iterations, Some(12));
        assert_eq!(back.elapsed_ms, Some(35_000));
    }

    #[test]
    fn system_notice_node_roundtrip() {
        let node = SystemNoticeNode {
            node_id: "node-sn-1".into(),
            turn_id: sample_turn_id(),
            status: NodeStatus::Completed,
            created_at_ms: 5000,
            updated_at_ms: 5000,
            message: "Context was compacted.".into(),
            level: Some("info".into()),
            category: Some("compaction".into()),
            source_trace: None,
        };
        let json = serde_json::to_string(&node).unwrap();
        let back: SystemNoticeNode = serde_json::from_str(&json).unwrap();
        assert_eq!(back.message, "Context was compacted.");
    }

    // ── Node status ────────────────────────────────────────────────────

    #[test]
    fn node_status_serde_roundtrip() {
        let cases = vec![
            (NodeStatus::Pending, "pending"),
            (NodeStatus::Running, "running"),
            (NodeStatus::Completed, "completed"),
            (NodeStatus::Failed, "failed"),
            (NodeStatus::Cancelled, "cancelled"),
        ];
        for (status, expected_str) in cases {
            let json = serde_json::to_string(&status).unwrap();
            assert_eq!(json, format!("\"{expected_str}\""));
            let back: NodeStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(back, status);
        }
    }

    #[test]
    fn node_status_default_is_pending() {
        assert_eq!(NodeStatus::default(), NodeStatus::Pending);
    }

    // ── Tool category ──────────────────────────────────────────────────

    #[test]
    fn tool_category_serde_roundtrip() {
        let cases = vec![
            (ToolCategory::File, "file"),
            (ToolCategory::Shell, "shell"),
            (ToolCategory::Search, "search"),
            (ToolCategory::Web, "web"),
            (ToolCategory::Mcp, "mcp"),
            (ToolCategory::Interaction, "interaction"),
            (ToolCategory::SubAgent, "sub_agent"),
            (ToolCategory::Memory, "memory"),
            (ToolCategory::Planning, "planning"),
            (ToolCategory::Other, "other"),
        ];
        for (cat, expected_str) in cases {
            let json = serde_json::to_string(&cat).unwrap();
            assert_eq!(json, format!("\"{expected_str}\""));
            let back: ToolCategory = serde_json::from_str(&json).unwrap();
            assert_eq!(back, cat);
        }
    }

    // ── Small output policy ────────────────────────────────────────────

    #[test]
    fn small_output_policy_constants() {
        assert_eq!(SMALL_OUTPUT_MAX_BYTES, 8_000);
        assert_eq!(SMALL_OUTPUT_MAX_LINES, 200);
        assert_eq!(SMALL_OUTPUT_MAX_TOKENS, 2_000);
    }

    #[test]
    fn is_small_output_within_limits() {
        assert!(is_small_output(500, 10, 200, false));
    }

    #[test]
    fn is_small_output_exceeds_bytes() {
        assert!(!is_small_output(9_000, 10, 200, false));
    }

    #[test]
    fn is_small_output_exceeds_lines() {
        assert!(!is_small_output(500, 300, 200, false));
    }

    #[test]
    fn is_small_output_exceeds_tokens() {
        assert!(!is_small_output(500, 10, 3_000, false));
    }

    #[test]
    fn is_small_output_binary_rejected() {
        assert!(!is_small_output(100, 5, 50, true));
    }

    // ── Materialized roundtrip: event → node recognition ───────────────

    #[test]
    fn text_delta_event_yields_text_node_on_roundtrip() {
        let event = TurnTimelineEvent {
            id: TimelineEventId::new("evt-text-1"),
            session_id: sample_session_id(),
            turn_id: sample_turn_id(),
            seq: 5,
            event_type: TimelineEventType::AssistantTextSnapshot,
            schema_version: TIMELINE_SCHEMA_VERSION,
            payload_json: serde_json::json!({
                "node_id": "node-at-1",
                "content": "Here is the answer.",
                "byte_length": 18
            }),
            created_at_ms: 2000,
        };

        let event_json = serde_json::to_string(&event).unwrap();
        let event_back: TurnTimelineEvent = serde_json::from_str(&event_json).unwrap();
        assert_eq!(
            event_back.event_type,
            TimelineEventType::AssistantTextSnapshot
        );

        let payload: AssistantTextSnapshotPayload =
            serde_json::from_value(event_back.payload_json).unwrap();
        let node = AssistantTextNode {
            node_id: payload.node_id,
            turn_id: event_back.turn_id,
            status: NodeStatus::Completed,
            created_at_ms: event_back.created_at_ms,
            updated_at_ms: event_back.created_at_ms,
            content: payload.content,
            byte_length: payload.byte_length,
            text_role: None,
            source_trace: Some(SourceEventTrace {
                event_ids: vec![event_back.id.to_string()],
                min_seq: Some(event_back.seq),
                max_seq: Some(event_back.seq),
            }),
        };

        assert_eq!(node.content, "Here is the answer.");
        assert_eq!(node.byte_length, 18);
    }

    #[test]
    fn tool_event_yields_tool_step_node_on_roundtrip() {
        let event = TurnTimelineEvent {
            id: TimelineEventId::new("evt-tool-1"),
            session_id: sample_session_id(),
            turn_id: sample_turn_id(),
            seq: 12,
            event_type: TimelineEventType::ToolCallFinished,
            schema_version: TIMELINE_SCHEMA_VERSION,
            payload_json: serde_json::json!({
                "call_id": "tc-abc",
                "tool_name": "grep",
                "success": true,
                "duration_ms": 120,
                "output_preview": {
                    "content": "src/main.rs:5: fn main()",
                    "byte_length": 23,
                    "line_count": 1,
                    "estimated_tokens": 6,
                    "is_binary": false,
                    "content_type": "search_results"
                },
                "output_detail": null,
                "error_message": null
            }),
            created_at_ms: 5000,
        };

        let event_json = serde_json::to_string(&event).unwrap();
        let event_back: TurnTimelineEvent = serde_json::from_str(&event_json).unwrap();

        let payload: ToolCallFinishedPayload =
            serde_json::from_value(event_back.payload_json).unwrap();
        assert!(payload.success);
        assert_eq!(payload.call_id, "tc-abc");
        assert_eq!(payload.duration_ms, Some(120));

        let preview = payload.output_preview.unwrap();
        assert_eq!(preview.content_type.unwrap(), "search_results");
        assert!(!preview.is_binary);
    }

    #[test]
    fn terminal_status_roundtrip_via_turn_finished() {
        let event = TurnTimelineEvent {
            id: TimelineEventId::new("evt-end-1"),
            session_id: sample_session_id(),
            turn_id: sample_turn_id(),
            seq: 100,
            event_type: TimelineEventType::TurnFinished,
            schema_version: TIMELINE_SCHEMA_VERSION,
            payload_json: serde_json::json!({
                "end_reason": "cancelled",
                "diagnosis_code": null,
                "severity": null,
                "user_message": "Turn was cancelled by user.",
                "iterations": 1,
                "tool_calls": 2,
                "elapsed_ms": 8000
            }),
            created_at_ms: 8000,
        };

        let json = serde_json::to_string(&event).unwrap();
        let back: TurnTimelineEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(back.event_type, TimelineEventType::TurnFinished);
        assert_eq!(back.seq, 100);

        let payload: TurnFinishedPayload = serde_json::from_value(back.payload_json).unwrap();
        assert_eq!(payload.end_reason, "cancelled");
        assert_eq!(payload.user_message.unwrap(), "Turn was cancelled by user.");
    }

    // ── ToolTargetMetadata ─────────────────────────────────────────────

    #[test]
    fn tool_target_metadata_default_is_empty() {
        let target = ToolTargetMetadata::default();
        assert!(target.path.is_none());
        assert!(target.command.is_none());
        assert!(target.url.is_none());
        assert!(target.query.is_none());
        assert!(target.mcp_server.is_none());
        assert!(target.label.is_none());
    }

    #[test]
    fn tool_target_metadata_all_fields_roundtrip() {
        let target = ToolTargetMetadata {
            path: Some("/etc/hosts".into()),
            command: Some("cat /etc/hosts".into()),
            url: Some("https://example.com".into()),
            query: Some("TODO".into()),
            mcp_server: Some("filesystem".into()),
            label: Some("Read system file".into()),
        };
        let json = serde_json::to_string(&target).unwrap();
        let back: ToolTargetMetadata = serde_json::from_str(&json).unwrap();
        assert_eq!(back.path.unwrap(), "/etc/hosts");
        assert_eq!(back.mcp_server.unwrap(), "filesystem");
    }
}
