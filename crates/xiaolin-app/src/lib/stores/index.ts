import { saveUIStateFromMeta } from "./persistence";
import { useChatMetaStore as _chatMetaStore } from "./chat-meta-store";

export { useConfigStore } from "./config-store";
export * from "./types";

export { useChatMetaStore } from "./chat-meta-store";
export { useStreamStore, EMPTY_STREAM } from "./stream-store";
export { useQueueStore } from "./queue-store";
export { useProjectStore } from "./project-store";
export { useGitStore, initGitStore, destroyGitStore } from "./git-store";
export { useUIStore, DEFAULT_SIDEBAR_WIDTH, MIN_SIDEBAR_WIDTH, MAX_SIDEBAR_WIDTH } from "./ui-store";
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

_chatMetaStore.subscribe((state, prev) => {
  if (state.chats !== prev.chats || state.activeChatId !== prev.activeChatId) {
    saveUIStateFromMeta(state.activeChatId, state.chats);
  }
});
