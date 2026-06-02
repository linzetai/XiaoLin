import { useChatMetaStore } from "./chat-meta-store";
import { useStreamStore, EMPTY_STREAM } from "./stream-store";
import { useQueueStore } from "./queue-store";
import type { ChatMeta, ChatUsage, StreamItem, SubAgentRunUI, QueuedMessage, ChatStreamSegment } from "./types";

const EMPTY_SUB_AGENT_RUNS: Record<string, SubAgentRunUI> = {};
const EMPTY_QUEUE: QueuedMessage[] = [];
const EMPTY_SEGMENTS: ChatStreamSegment[] = [];

export function useActiveChatId(): string {
  return useChatMetaStore((s) => s.activeChatId);
}

export function useActiveChatMeta(): ChatMeta | undefined {
  return useChatMetaStore((s) => s.chats[s.activeChatId]);
}

export function useActiveStream(): StreamItem[] {
  const activeChatId = useChatMetaStore((s) => s.activeChatId);
  return useStreamStore((s) => s.streams[activeChatId] ?? EMPTY_STREAM);
}

export function useChatStream(chatId: string): StreamItem[] {
  return useStreamStore((s) => s.streams[chatId] ?? EMPTY_STREAM);
}

export function useChatUsage(chatId: string): ChatUsage | undefined {
  return useStreamStore((s) => s.usage[chatId]);
}

export function useActiveSubAgentRuns(): Record<string, SubAgentRunUI> {
  const activeChatId = useChatMetaStore((s) => s.activeChatId);
  return useStreamStore((s) => s.subAgentRuns[activeChatId] ?? EMPTY_SUB_AGENT_RUNS);
}

export function useChatSubAgentRuns(chatId: string): Record<string, SubAgentRunUI> {
  return useStreamStore((s) => s.subAgentRuns[chatId] ?? EMPTY_SUB_AGENT_RUNS);
}

export function useChatLastSegments(chatId: string): ChatStreamSegment[] {
  return useStreamStore((s) => s.lastSegments[chatId] ?? EMPTY_SEGMENTS);
}

export function useChatQueue(chatId: string): QueuedMessage[] {
  return useQueueStore((s) => s.queues[chatId] ?? EMPTY_QUEUE);
}
