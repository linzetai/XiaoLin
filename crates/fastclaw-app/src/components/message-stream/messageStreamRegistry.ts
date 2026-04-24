import type { ToolCall } from "./ToolCallCard";

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
}

export const detachedStreams = new Map<string, DetachedStream>();
export const MAX_DETACHED_STREAMS = 64;
