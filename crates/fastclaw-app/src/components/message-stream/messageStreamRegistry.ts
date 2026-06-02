import type { ToolCall } from "./ToolCallCard";

export interface DetachedInteraction {
  type: "approval_required" | "ask_question";
  data: Record<string, unknown>;
}

export interface DetachedStream {
  agentId: string;
  chatId: string;
  acc: string;
  toolCalls: ToolCall[];
  done: boolean;
  error: boolean;
  sessionId?: string;
  scrollPosition?: number;
  cleanup: () => void;
  pendingInteraction?: DetachedInteraction;
  needsAttention: boolean;
}

export const detachedStreams = new Map<string, DetachedStream>();
export const MAX_DETACHED_STREAMS = 64;
