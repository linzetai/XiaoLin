export interface Agent {
  id: string;
  name: string;
  initial: string;
  color: string;
  tagline: string;
  online: boolean;
  model: string;
  avatar?: string;
}

export interface ChatMessageToolCall {
  id: string;
  name: string;
  status: "running" | "success" | "error";
  args?: string;
  result?: string;
  displayOutput?: string;
  duration?: number;
  metadata?: Record<string, unknown> | null;
  /** When true, `result`/`displayOutput` are truncated and the full text must be lazy-loaded via `getToolOutput`. */
  truncated?: boolean;
  /** Original (untruncated) length of `result` in chars, for display. */
  fullLength?: number;
  /** Message row id in the DB — used to locate the tool call for lazy-load. */
  messageId?: number;
  /** Session id — needed to call `getToolOutput`. */
  sessionId?: string;
  /** Output asset handle for handle-based lazy-load (Phase 10). */
  outputHandle?: string;
  /** Size class: "small" | "medium" | "large". */
  outputSizeClass?: string;
  /** Whether expansion through the handle API is available. */
  outputIsExpandable?: boolean;
}

export interface ChatMessageImage {
  url: string;
  alt?: string;
}

export interface QueuedMention {
  type: "file" | "dir" | "skill";
  id: string;
  label: string;
}

export interface QueuedMessage {
  id: string;
  content: string;
  mentions: QueuedMention[];
  images: ChatMessageImage[];
  createdAt: Date;
  status: "pending" | "sending" | "failed";
  error?: string;
}

export interface ChatMessage {
  role: "user" | "assistant" | "system";
  content: string;
  id: number;
  /** Persisted DB message id, present for messages restored from session history. */
  backendId?: number;
  timestamp: Date;
  chatId: string;
  toolCalls?: ChatMessageToolCall[];
  images?: ChatMessageImage[];
  usage?: ChatUsage;
  isSteer?: boolean;
  metadata?: Record<string, unknown>;
}

export interface SubAgentToolCall {
  id: string;
  name: string;
  status: "running" | "success" | "error";
  args?: string;
  result?: string;
}

export interface SubAgentNotification {
  message: string;
  timestamp: number;
}

export interface SubAgentRunUI {
  runId: string;
  agentId: string;
  subagentType: string;
  task: string;
  depth: number;
  status: "pending" | "running" | "completed" | "failed" | "cancelled";
  content: string;
  toolCalls: SubAgentToolCall[];
  result?: string;
  toolCallsMade: number;
  iterations: number;
  elapsedMs?: number;
  notifications: SubAgentNotification[];
}

export interface ChatStreamSegment {
  id: string;
  type: "text" | "tool" | "reasoning" | "iteration_boundary";
  content?: string;
  toolCall?: ChatMessageToolCall;
  iteration?: number;
}

export interface BriefMessageData {
  id: string;
  content: string;
  mode: "normal" | "proactive";
  timestamp: number;
}

export type StreamItem =
  | { type: "message"; data: ChatMessage }
  | { type: "brief"; data: BriefMessageData };

export interface ChatUsage {
  promptTokens: number;
  completionTokens: number;
  totalTokens: number;
  elapsedMs: number;
  contextTokens?: number;
  contextWindow?: number;
}

export type ExecutionMode = "agent" | "plan";

export type ModeSource = "request" | "registry" | "default";

export type PlanStepStatus = "pending" | "in_progress" | "completed";

export interface PlanStep {
  step: string;
  status: PlanStepStatus;
}

export interface PlanUpdateData {
  explanation?: string;
  steps: PlanStep[];
}

export type GoalStatus =
  | "active"
  | "completed"
  | "failed"
  | "cancelled"
  | "paused"
  | "budget_limited";

export interface GoalData {
  id: string;
  description: string;
  status: GoalStatus | string;
  token_budget?: number;
  tokens_used: number;
  time_used_seconds: number;
  pause_reason?: string;
  continuation_rounds: number;
  created_at: number;
  updated_at: number;
}

export interface ChatMeta {
  id: string;
  localKey: string;
  title: string;
  workDir: string | null;
  projectId: string | null;
  source: string;
  createdAt: Date;
  messageCount: number;
  open: boolean;
  executionMode: ExecutionMode;
  planFilePath?: string;
  planFileExists?: boolean;
  planApprovalPending?: boolean;
}

export interface Chat {
  id: string;
  localKey: string;
  title: string;
  workDir: string | null;
  projectId: string | null;
  source: string;
  stream: StreamItem[];
  createdAt: Date;
  messageCount: number;
  open: boolean;
  usage?: ChatUsage;
  subAgentRuns: Record<string, SubAgentRunUI>;
  executionMode: ExecutionMode;
  planFilePath?: string;
  planFileExists?: boolean;
  lastSegments?: ChatStreamSegment[];
}

export interface BackendSession {
  id: string;
  agentId: string;
  title: string | null;
  workDir?: string | null;
  projectId?: string | null;
  source?: string;
  messageCount: number;
  createdAt: string;
  updatedAt: string;
  totalPromptTokens?: number;
  totalCompletionTokens?: number;
  totalElapsedMs?: number;
}

export interface BackendMessage {
  id: number;
  role: string;
  content: unknown;
  name: string | null;
  toolCallId: string | null;
  toolCallsJson?: Array<{
    id: string;
    type: string;
    function: { name: string; arguments: string };
    output?: string;
    display_output?: string;
    success?: boolean;
    duration_ms?: number;
    metadata?: Record<string, unknown>;
    truncated?: boolean;
    full_length?: number;
    output_handle?: string;
    output_size_class?: string;
    output_is_expandable?: boolean;
  }> | null;
  createdAt: string;
  reasoningContent?: string | null;
  promptTokens?: number;
  completionTokens?: number;
  totalTokens?: number;
  elapsedMs?: number;
  segmentOrder?: string[] | null;
}
