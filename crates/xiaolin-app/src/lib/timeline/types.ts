// Canonical turn timeline types for UI-visible chat transcript state.
//
// These types mirror the Rust protocol definitions in
// crates/xiaolin-protocol/src/timeline.rs. Both live WebSocket rendering and
// history replay consume these same types through a single reducer contract.

// ============================================================================
// Timeline event identity
// ============================================================================

/** Globally unique, idempotent identifier for a timeline event. */
export type TimelineEventId = string;

// ============================================================================
// Schema version
// ============================================================================

/** Current schema version for timeline events. */
export const TIMELINE_SCHEMA_VERSION = 2;

// ============================================================================
// Reasoning visibility
// ============================================================================

/**
 * Controls whether reasoning content is public or private.
 *
 * Private reasoning MUST NOT be persisted, emitted over WebSocket, served
 * via history/reconnect APIs, or rendered by the frontend.
 */
export type ReasoningVisibility = "public" | "private";

// ============================================================================
// Assistant text role
// ============================================================================

/**
 * Distinguishes between public activity narration and the final answer.
 */
export type AssistantTextRole = "activity" | "final";

// ============================================================================
// Timeline event type enum
// ============================================================================

/**
 * Discriminant for timeline events.
 *
 * Each variant maps to a specific lifecycle or content moment in a turn.
 */
export type TimelineEventType =
  | "turn_started"
  | "user_message_created"
  | "assistant_text_delta"
  | "assistant_text_snapshot"
  | "reasoning_delta"
  | "reasoning_snapshot"
  | "tool_call_started"
  | "tool_call_progress"
  | "tool_call_finished"
  | "approval_requested"
  | "approval_resolved"
  | "iteration_boundary"
  | "assistant_message_finalized"
  | "turn_finished"
  | "compact_boundary"
  | "system_notice";

// ============================================================================
// Timeline event payload structs
// ============================================================================

export interface TurnStartedPayload {
  session_id?: string;
  execution_mode?: string;
  agent_id?: string;
}

export interface UserMessageCreatedPayload {
  message_id?: string;
  /** Client-generated id echoed back for optimistic overlay reconciliation. */
  client_message_id?: string;
  content: string;
  attachments?: string[];
}

export interface AssistantTextDeltaPayload {
  /** Target node id so deltas for the same text stream can be coalesced. */
  node_id: string;
  /** The delta content. */
  delta: string;
  /** Byte offset from the start of the text stream. */
  offset?: number;
  /** Whether this is activity narration or final answer text. */
  text_role?: AssistantTextRole;
}

export interface AssistantTextSnapshotPayload {
  /** The node id this snapshot represents. */
  node_id: string;
  /** The full text content at the time of the snapshot. */
  content: string;
  /** Byte length of the content. */
  byte_length?: number;
  /** Whether this is activity narration or final answer text. */
  text_role?: AssistantTextRole;
}

export interface ReasoningDeltaPayload {
  node_id: string;
  delta: string;
  offset?: number;
  /** Visibility control: private reasoning MUST NOT be persisted or emitted. */
  visibility?: ReasoningVisibility;
}

export interface ReasoningSnapshotPayload {
  node_id: string;
  content: string;
  /** Visibility control: private reasoning MUST NOT be persisted or emitted. */
  visibility?: ReasoningVisibility;
}

export interface ToolCallStartedPayload {
  call_id: string;
  tool_name: string;
  tool_category?: string;
  display_title?: string;
  target?: ToolTargetMetadata;
  args?: string;
}

export interface ToolCallProgressPayload {
  call_id: string;
  message: string;
  progress?: number; // [0.0, 1.0]
  partial_output?: string;
}

export interface ToolCallFinishedPayload {
  call_id: string;
  tool_name: string;
  success: boolean;
  duration_ms?: number;
  output_preview?: OutputPreview;
  output_detail?: OutputDetailReference;
  error_message?: string;
}

export interface ApprovalRequestedPayload {
  approval_id: string;
  action: string;
  reason: string;
  risk_level?: string;
}

export interface ApprovalResolvedPayload {
  approval_id: string;
  decision: string;
  source: string;
}

export interface IterationBoundaryPayload {
  iteration: number;
}

export interface AssistantMessageFinalizedPayload {
  text_node_id?: string;
  final_text_content?: string;
}

export interface TurnFinishedPayload {
  end_reason: string;
  diagnosis_code?: string;
  severity?: string;
  user_message?: string;
  iterations?: number;
  tool_calls?: number;
  elapsed_ms?: number;
  /** Number of repeated force stops during the turn. */
  repeated_force_stops?: number;
  /** Number of repeated warnings during the turn. */
  repeated_warns?: number;
  /** Number of consecutive rounds with no progress. */
  no_progress_count?: number;
}

export interface CompactBoundaryPayload {
  trigger: string;
  pre_compact_tokens: number;
  post_compact_tokens: number;
  messages_removed: number;
}

export interface SystemNoticePayload {
  message: string;
  level?: string;
  category?: string;
}

// ============================================================================
// Tool target metadata
// ============================================================================

export interface ToolTargetMetadata {
  path?: string;
  command?: string;
  url?: string;
  query?: string;
  mcp_server?: string;
  label?: string;
}

// ============================================================================
// Output preview and detail reference
// ============================================================================

export interface OutputPreview {
  /** The full text output (when small). */
  content: string;
  byte_length: number;
  line_count: number;
  estimated_tokens: number;
  is_binary?: boolean;
  content_type?: string;
}

export interface OutputDetailReference {
  /** The session-scoped tool output handle string. */
  handle: string;
  byte_length: number;
  line_count: number;
  is_expandable?: boolean;
  size_class?: string;
  summary?: string;
  content_type?: string;
}

// ============================================================================
// Small-output policy constants
// ============================================================================

export const SMALL_OUTPUT_MAX_BYTES = 8_000;
export const SMALL_OUTPUT_MAX_LINES = 200;
export const SMALL_OUTPUT_MAX_TOKENS = 2_000;

export function isSmallOutput(
  byteLength: number,
  lineCount: number,
  estimatedTokens: number,
  isBinary: boolean,
): boolean {
  return (
    !isBinary &&
    byteLength <= SMALL_OUTPUT_MAX_BYTES &&
    lineCount <= SMALL_OUTPUT_MAX_LINES &&
    estimatedTokens <= SMALL_OUTPUT_MAX_TOKENS
  );
}

// ============================================================================
// Source event trace metadata
// ============================================================================

export interface SourceEventTrace {
  event_ids: string[];
  min_seq?: number;
  max_seq?: number;
}

// ============================================================================
// The canonical timeline event
// ============================================================================

export interface TurnTimelineEvent {
  id: TimelineEventId;
  session_id: string;
  turn_id: string;
  seq: number;
  event_type: TimelineEventType;
  schema_version: number;
  payload_json: Record<string, unknown>;
  created_at_ms: number;
}

// ============================================================================
// Node status
// ============================================================================

export type NodeStatus = "pending" | "running" | "completed" | "failed" | "cancelled";

// ============================================================================
// Tool category
// ============================================================================

export type ToolCategory =
  | "file"
  | "shell"
  | "search"
  | "web"
  | "mcp"
  | "interaction"
  | "sub_agent"
  | "memory"
  | "planning"
  | "other";

// ============================================================================
// Display node types
// ============================================================================

export type TurnDisplayNode =
  | UserMessageNode
  | AssistantTextNode
  | ReasoningNode
  | ToolStepNode
  | ToolGroupNode
  | ApprovalNode
  | IterationBoundaryNode
  | TurnStatusNode
  | SystemNoticeNode;

/** Discriminant type for TurnDisplayNode["kind"]. */
export type TurnDisplayNodeKind = TurnDisplayNode["kind"];

export interface UserMessageNode {
  kind: "user_message";
  node_id: string;
  turn_id: string;
  status: NodeStatus;
  created_at_ms: number;
  updated_at_ms: number;
  content: string;
  message_id?: string;
  attachments?: string[];
  source_trace?: SourceEventTrace;
}

export interface AssistantTextNode {
  kind: "assistant_text";
  node_id: string;
  turn_id: string;
  status: NodeStatus;
  created_at_ms: number;
  updated_at_ms: number;
  content: string;
  byte_length?: number;
  /** Whether this is activity narration or final answer text. */
  text_role?: AssistantTextRole;
  source_trace?: SourceEventTrace;
}

export interface ReasoningNode {
  kind: "reasoning";
  node_id: string;
  turn_id: string;
  status: NodeStatus;
  created_at_ms: number;
  updated_at_ms: number;
  content: string;
  collapsed?: boolean;
  /** Visibility control: private reasoning must not be rendered. */
  visibility?: ReasoningVisibility;
  source_trace?: SourceEventTrace;
}

export interface ToolStepNode {
  kind: "tool_step";
  node_id: string;
  turn_id: string;
  status: NodeStatus;
  created_at_ms: number;
  updated_at_ms: number;
  tool_name: string;
  tool_category?: ToolCategory;
  display_title: string;
  call_id: string;
  target?: ToolTargetMetadata;
  progress_label?: string;
  progress?: number;
  started_at_ms?: number;
  finished_at_ms?: number;
  duration_ms?: number;
  output_preview?: OutputPreview;
  output_detail?: OutputDetailReference;
  error_message?: string;
  args?: string;
  source_trace?: SourceEventTrace;
}

export interface ToolGroupNode {
  kind: "tool_group";
  node_id: string;
  turn_id: string;
  status: NodeStatus;
  created_at_ms: number;
  updated_at_ms: number;
  group_label: string;
  step_count: number;
  steps: ToolStepNode[];
  collapsed?: boolean;
  source_trace?: SourceEventTrace;
}

export interface ApprovalNode {
  kind: "approval";
  node_id: string;
  turn_id: string;
  status: NodeStatus;
  created_at_ms: number;
  updated_at_ms: number;
  approval_id: string;
  action: string;
  reason: string;
  risk_level?: string;
  decision?: string;
  decision_source?: string;
  source_trace?: SourceEventTrace;
}

export interface IterationBoundaryNode {
  kind: "iteration_boundary";
  node_id: string;
  turn_id: string;
  status: NodeStatus;
  created_at_ms: number;
  updated_at_ms: number;
  iteration: number;
  source_trace?: SourceEventTrace;
}

export interface TurnStatusNode {
  kind: "turn_status";
  node_id: string;
  turn_id: string;
  status: NodeStatus;
  created_at_ms: number;
  updated_at_ms: number;
  end_reason: string;
  summary?: string;
  diagnosis?: TerminalDiagnosisMetadata;
  elapsed_ms?: number;
  source_trace?: SourceEventTrace;
}

export interface SystemNoticeNode {
  kind: "system_notice";
  node_id: string;
  turn_id: string;
  status: NodeStatus;
  created_at_ms: number;
  updated_at_ms: number;
  message: string;
  level?: string;
  category?: string;
  source_trace?: SourceEventTrace;
}

// ============================================================================
// Terminal diagnosis metadata
// ============================================================================

export interface TerminalDiagnosisMetadata {
  diagnosis_code?: string;
  severity?: string;
  user_message?: string;
  iterations?: number;
  tool_calls?: number;
  repeated_force_stops?: number;
  repeated_warns?: number;
  no_progress_count?: number;
}

// ============================================================================
// Timeline state (accumulator for the reducer)
// ============================================================================

/** Identity info tracked in the nodeIdIndex for conflict detection. */
export interface NodeIdentityInfo {
  kind: TurnDisplayNodeKind;
  turnId: string;
  visibility?: ReasoningVisibility;
  textRole?: AssistantTextRole;
}

export interface TimelineState {
  /** All events in order by seq. */
  events: TurnTimelineEvent[];
  /** Materialized display nodes. */
  nodes: TurnDisplayNode[];
  /** The maximum seq seen so far. */
  maxSeq: number;
  /** The session id. */
  sessionId: string;
  /** Per-turn node index: turn_id → node_ids. */
  turnIndex: Record<string, string[]>;
  /** Per-node event trace accumulator. */
  eventTraces: Record<string, string[]>;
  /**
   * Node identity index: node_id → identity info.
   * Survives history reload/replay. Used to detect protocol violations
   * when an event tries to change a node's kind, turn, visibility, or role.
   */
  nodeIdIndex: Record<string, NodeIdentityInfo>;
}

/** Empty initial state for a session. */
export function emptyTimelineState(sessionId: string): TimelineState {
  return {
    events: [],
    nodes: [],
    maxSeq: 0,
    sessionId,
    turnIndex: {},
    eventTraces: {},
    nodeIdIndex: {},
  };
}

// ============================================================================
// Source tracking
// ============================================================================

/**
 * Tracks the provenance of timeline data for a session.
 *
 * - "none": No data loaded yet.
 * - "probing": Initial probe in flight (snapshot + buffered WS events).
 * - "authoritative_pending_snapshot": WS events received but snapshot not yet complete.
 * - "authoritative": Canonical timeline events from the server.
 * - "legacy": Synthesized from old message format (degraded experience).
 */
export type TimelineSource =
  | "none"
  | "probing"
  | "authoritative_pending_snapshot"
  | "authoritative"
  | "legacy";

// ============================================================================
// Ephemeral overlay types
// ============================================================================

/** Optimistic user message shown before the authoritative event arrives. */
export interface ProvisionalUserMessage {
  clientMessageId: string;
  localTurnId: string;
  content: string;
  attachments?: string[];
  createdAtMs: number;
  status: "sending" | "failed";
}

/** Live partial output patch for a running tool — not durable. */
export interface ToolOutputPatch {
  callId: string;
  content: string;
  truncated: boolean;
  updatedAtMs: number;
}

// ============================================================================
// Session timeline record (store envelope)
// ============================================================================

/**
 * Per-session envelope that wraps canonical state, source tracking,
 * gap management metadata, and ephemeral overlays.
 */
export interface SessionTimelineRecord {
  /** Provenance of the current timeline data. */
  source: TimelineSource;
  /** When the probe started (ms). */
  probeStartedAtMs: number;
  /** When the probe completed (ms). */
  probeCompletedAtMs: number;
  /** The canonical timeline state (events + nodes + indices). */
  canonical: TimelineState;
  /** All known events by id (runtime-only, not serialized). */
  knownById: Map<string, TurnTimelineEvent>;
  /** Events in the contiguous applied prefix. */
  appliedEvents: TurnTimelineEvent[];
  /** Events received out-of-order, waiting for gap fill (runtime-only). */
  pendingBySeq: Map<number, TurnTimelineEvent>;
  /** The last contiguous sequence number applied. */
  lastContiguousSeq: number;
  /** Whether a gap fill request is in flight. */
  gapFillInFlight: boolean;
  /** Optimistic user messages awaiting reconciliation. */
  optimisticUsers: Record<string, ProvisionalUserMessage>;
  /** Live tool output patches for running tools. */
  toolOutputPatches: Record<string, ToolOutputPatch>;
}
