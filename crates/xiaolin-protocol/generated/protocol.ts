/**
 * Auto-generated TypeScript types from xiaolin-protocol Rust crate.
 *
 * DO NOT EDIT MANUALLY — regenerate with `cargo run -p xiaolin-protocol --bin ts-codegen`.
 *
 * These types mirror the Rust protocol types 1:1 and are the single source
 * of truth for frontend/backend contract.
 */

// ── ID Types (all transparent newtypes over string) ─────────────────

export type AgentId = string;
export type SessionId = string;
export type TurnId = string;
export type SubmissionId = string;
export type MessageId = string;
export type ToolCallId = string;

// ── Token Usage ─────────────────────────────────────────────────────

export interface TokenUsage {
  prompt_tokens: number;
  completion_tokens: number;
  total_tokens: number;
}

// ── Message Types ───────────────────────────────────────────────────

export type Role = "system" | "user" | "assistant" | "tool";

export type ContentPart =
  | { type: "text"; text: string }
  | { type: "image_url"; image_url: { url: string; detail?: string } };

export type MessagePhase = "thinking" | "responding" | "done";

export type CompactTrigger = "manual" | "auto" | "threshold";

export type ExecutionMode =
  | "autonomous"
  | "supervised"
  | "plan_only"
  | "plan_and_execute";

export interface AskQuestionOption {
  id: string;
  label: string;
}

export type MessageTarget = "user" | "agent" | "system" | "broadcast";

// ── Tool Types ──────────────────────────────────────────────────────

export type ToolKind = "builtin" | "mcp" | "custom";

export interface ToolParameterSchema {
  type: string;
  properties?: Record<string, unknown>;
  required?: string[];
  description?: string;
}

export interface FunctionDefinition {
  name: string;
  description?: string;
  parameters?: ToolParameterSchema;
}

export interface ToolDefinition {
  kind: ToolKind;
  function: FunctionDefinition;
  source?: string;
}

// ── Tool Call Data (used in TurnEnd) ────────────────────────────────

export interface ToolCallFunction {
  name: string;
  arguments: string;
}

export interface ToolCallData {
  id: string;
  call_type: string;
  function: ToolCallFunction;
  output?: string;
  success?: boolean;
  duration_ms?: number;
}

// ── Turn Summary ────────────────────────────────────────────────────

export interface TurnSummary {
  turn_id: TurnId;
  tool_calls_made: number;
  iterations: number;
  usage?: TokenUsage;
  elapsed_ms: number;
  context_tokens?: number;
  context_window?: number;
}

// ── Context Warning Level ───────────────────────────────────────────

export type ContextWarningLevel = "soft" | "hard";

// ── Approval Types ──────────────────────────────────────────────────

export type ApprovalDecision =
  | { decision: "approved" }
  | { decision: "approved_for_session" }
  | { decision: "denied" }
  | { decision: "timed_out" }
  | { decision: "abort" };

export type PendingAction =
  | { action_type: "shell_command"; command: string; cwd: string }
  | { action_type: "file_write"; path: string }
  | { action_type: "apply_patch"; paths: string[] }
  | { action_type: "network_access"; host: string; port: number };

// ── Abort Reason ─────────────────────────────────────────────────────
export type AbortReason = "interrupted" | "replaced" | "budget_limited";

// ── Error Code ──────────────────────────────────────────────────────
export type ErrorCode =
  | "context_window_exceeded"
  | "usage_limit_exceeded"
  | "server_overloaded"
  | "connection_failed"
  | "stream_disconnected"
  | "sandbox_error"
  | "unauthorized"
  | "bad_request"
  | "other";

// ── Warning Category ────────────────────────────────────────────────
export type WarningCategory = "budget" | "context_pressure" | "tool_failure";

// ── Turn Context Item ───────────────────────────────────────────────
export interface TurnContextItem {
  turn_id: TurnId;
  cwd?: string;
  model: string;
  execution_mode: ExecutionMode;
  agent_id: string;
}

// ── AgentEvent (discriminated union) ────────────────────────────────

export type AgentEvent =
  | { type: "turn_start"; turn_id: TurnId; session_id?: string }
  | { type: "turn_end"; turn_id: TurnId; summary: TurnSummary; session_id?: string; final_tool_calls?: ToolCallData[] }
  | { type: "content_delta"; turn_id: TurnId; delta: unknown }
  | { type: "reasoning_delta"; turn_id: TurnId; content: string }
  | { type: "tool_executing"; turn_id: TurnId; tool_name: string; call_id: string; args?: string }
  | { type: "tool_result"; turn_id: TurnId; tool_name: string; call_id: string; output: string; display_output?: string; success: boolean; metadata?: unknown }
  | { type: "tool_progress"; turn_id: TurnId; tool_name: string; call_id: string; message: string; progress?: number; partial_output?: string }
  | { type: "ask_question"; turn_id: TurnId; request_id: string; question: string; options: AskQuestionOption[]; timeout_secs?: number; allow_multiple?: boolean }
  | { type: "context_usage_update"; turn_id: TurnId; used_tokens: number; limit_tokens: number; compressed: boolean; tokens_saved?: number }
  | { type: "context_warning"; turn_id: TurnId; level: ContextWarningLevel; used_tokens: number; limit_tokens: number; message: string }
  | { type: "compact_boundary"; turn_id: TurnId; trigger: CompactTrigger; pre_compact_tokens: number; post_compact_tokens: number; messages_removed: number }
  | { type: "mode_change"; turn_id: TurnId; from: ExecutionMode; to: ExecutionMode }
  | { type: "plan_file_update"; turn_id: TurnId; session_id: string; path: string; exists: boolean }
  | { type: "suggestions"; turn_id: TurnId; items: string[] }
  | { type: "brief_message"; turn_id: TurnId; content: string; attachments: string[]; mode: string }
  | { type: "sub_agent_start"; turn_id: TurnId; run_id: string; agent_id: string; subagent_type: string; task: string; depth: number }
  | { type: "sub_agent_delta"; turn_id: TurnId; run_id: string; content: string }
  | { type: "sub_agent_tool_executing"; turn_id: TurnId; run_id: string; tool_name: string; call_id: string; args?: string }
  | { type: "sub_agent_tool_result"; turn_id: TurnId; run_id: string; tool_name: string; call_id: string; output: string; success: boolean }
  | { type: "sub_agent_complete"; turn_id: TurnId; run_id: string; status: string; result?: string; tool_calls_made: number; iterations: number; usage?: TokenUsage; elapsed_ms?: number }
  | { type: "approval_required"; turn_id: TurnId; approval_id: string; action: PendingAction; reason: string; available_decisions: ApprovalDecision[] }
  | { type: "approval_resolved"; turn_id: TurnId; approval_id: string; decision: ApprovalDecision; source: string }
  | { type: "error"; turn_id: TurnId; message: string; error_code?: ErrorCode }
  | { type: "turn_aborted"; turn_id: TurnId; reason: AbortReason; completed_at?: string; duration_ms?: number }
  | { type: "stream_error"; turn_id: TurnId; message: string; error_code?: ErrorCode; retry_attempt: number }
  | { type: "warning"; turn_id: TurnId; message: string; category: WarningCategory }
  | { type: "memory_recall"; turn_id: TurnId; episode_count: number; injected_tokens: number };

// ── ClientOp (discriminated union) ──────────────────────────────────

export type ClientOp =
  | { type: "chat_submit"; session_id?: string; agent_id: string; messages: unknown[]; model_override?: string; request_id?: string; work_dir?: string }
  | { type: "chat_cancel" }
  | { type: "chat_resume"; session_id: string; message?: string }
  | { type: "answer_question"; request_id: string; answer: string }
  | { type: "compact"; session_id: string; trigger?: CompactTrigger }
  | { type: "session_list"; agent_id?: string }
  | { type: "session_get"; session_id: string }
  | { type: "session_messages"; session_id: string }
  | { type: "session_delete"; session_id: string }
  | { type: "session_clear"; session_id: string }
  | { type: "session_update_title"; session_id: string; title: string }
  | { type: "session_claim"; session_id: string }
  | { type: "session_export"; session_id: string }
  | { type: "session_import"; data: unknown }
  | { type: "config_get"; key?: string }
  | { type: "config_set"; key: string; value: unknown }
  | { type: "mcp_list" }
  | { type: "mcp_reload"; server_id?: string }
  | { type: "mcp_remove"; server_id: string }
  | { type: "mcp_add"; server_id: string; config: unknown }
  | { type: "mcp_status" }
  | { type: "agent_list" }
  | { type: "agent_get"; agent_id: string }
  | { type: "agent_create"; agent_id: string; config: unknown }
  | { type: "agent_update"; agent_id: string; config: unknown }
  | { type: "agent_delete"; agent_id: string }
  | { type: "tool_list"; agent_id?: string }
  | { type: "skill_list" }
  | { type: "skill_install"; source: string }
  | { type: "mode_set"; mode: ExecutionMode; session_id?: string }
  | { type: "mode_get"; session_id?: string }
  | { type: "subscribe"; topic: string }
  | { type: "unsubscribe"; topic: string }
  | { type: "ping" };

// ── HistoryItem (model-visible conversation history) ────────────────

export type HistoryItem =
  | { type: "message"; role: Role; content: unknown; name?: string }
  | { type: "tool_use"; tool_call_id: string; tool_name: string; arguments: string; output: string; success: boolean; duration_ms?: number }
  | { type: "compact_boundary"; trigger: CompactTrigger; pre_tokens: number; post_tokens: number; removed_count: number }
  | { type: "turn_usage"; turn_id: TurnId; usage: TokenUsage };

// ── Envelope ────────────────────────────────────────────────────────

export interface Envelope<T> {
  id: SubmissionId;
  payload: T;
}

// ── Helper: type guard ──────────────────────────────────────────────

export function isAgentEvent(value: unknown): value is AgentEvent {
  return (
    typeof value === "object" &&
    value !== null &&
    "type" in value &&
    "turn_id" in value
  );
}

export function isTurnEnd(event: AgentEvent): event is Extract<AgentEvent, { type: "turn_end" }> {
  return event.type === "turn_end";
}

export function isError(event: AgentEvent): event is Extract<AgentEvent, { type: "error" }> {
  return event.type === "error";
}
