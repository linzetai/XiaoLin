import { create } from "zustand";
import { buildAgentSlice } from "./agent-store";
import { saveUIState, saveUIStateFromMeta } from "./persistence";
import { buildSessionSlice } from "./session-store";
import type { AgentState } from "./types";
import { buildUISlice } from "./ui-store";
import { useChatMetaStore as _chatMetaStore } from "./chat-meta-store";

export { useConfigStore } from "./config-store";
export { buildAgentSlice } from "./agent-store";
export { buildSessionSlice } from "./session-store";
export { buildUISlice } from "./ui-store";
export * from "./types";

export { useChatMetaStore } from "./chat-meta-store";
export { useStreamStore, EMPTY_STREAM } from "./stream-store";
export { useQueueStore } from "./queue-store";
export { useUIStore } from "./ui-store";
export {
  useActiveChatId,
  useActiveChatMeta,
  useActiveStream,
  useChatStream,
  useChatUsage,
  useActiveSubAgentRuns,
  useChatSubAgentRuns,
  useChatLastSegments,
  useChatQueue,
} from "./selectors";

/**
 * @deprecated Use useChatMetaStore, useStreamStore, useQueueStore, useUIStore instead.
 * Kept for backward compatibility during migration.
 */
export const useAgentStore = create<AgentState>((set, get) => ({
  ...buildSessionSlice({ set, get }),
  ...buildAgentSlice({ set, get }),
  ...buildUISlice(set),
}));

useAgentStore.subscribe((state, prev) => {
  if (state.agentChats !== prev.agentChats) {
    saveUIState(state);
  }
});

_chatMetaStore.subscribe((state, prev) => {
  if (state.chats !== prev.chats || state.activeChatId !== prev.activeChatId) {
    saveUIStateFromMeta(state.activeChatId, state.chats);
  }
});
