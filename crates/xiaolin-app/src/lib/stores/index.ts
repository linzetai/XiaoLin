import { saveUIStateFromMeta } from "./persistence";
import { useChatMetaStore as _chatMetaStore } from "./chat-meta-store";

export { useConfigStore } from "./config-store";
export { useLocaleStore } from "./locale-store";
export type { Locale, ResponseLang } from "./locale-store";
export { usePermissionStore } from "./permission-store";
export { useAutomationStore } from "./automation-store";
export { usePluginStore } from "./plugin-store";
export * from "./types";

export { useChatMetaStore } from "./chat-meta-store";
export { useGoalStore, initGoalListener, teardownGoalListener } from "./goal-store";
export {
  useFileViewerStore,
  initFileArtifactListener,
  teardownFileArtifactListener,
  reloadArtifactsForCurrentSession,
  resolveFilePath,
} from "./file-viewer-store";
export type { OpenFile, FileViewerState } from "./file-viewer-store";
export { useStreamStore, EMPTY_STREAM } from "./stream-store";
export { useTimelineStore } from "./timeline-store";
export { useQueueStore } from "./queue-store";
export { useProjectStore } from "./project-store";
export { useGitStore, initGitStore, destroyGitStore } from "./git-store";
export { useUIStore, DEFAULT_SIDEBAR_WIDTH, MIN_SIDEBAR_WIDTH, MAX_SIDEBAR_WIDTH } from "./ui-store";
export {
  useBrowserStore,
  initBrowserEvents,
  teardownBrowserEvents,
  shouldShowBrowserWebView,
  hasBrowserPages,
  browserGoBack,
  browserGoForward,
  browserReload,
  browserResizeWebview,
  normalizeNavUrl,
  isHttpsUrl,
  MAX_BROWSER_PAGES,
} from "./browser-store";
export type { BrowserPage, BrowserDownload, PageLoadState } from "./browser-store";
export { useSearchStore } from "./search-store";
export type { SearchResult, SearchFilters, SearchIndexStatus } from "./search-store";
export { useTerminalStore } from "./terminal-store";
export type { TerminalSession, TerminalLine } from "./terminal-store";
export { usePtyStore } from "./pty-store";
export type { PtySession } from "./pty-store";
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
  useActiveGoal,
  useChatGoal,
} from "./selectors";

_chatMetaStore.subscribe((state, prev) => {
  if (state.chats !== prev.chats || state.activeChatId !== prev.activeChatId) {
    saveUIStateFromMeta(state.activeChatId, state.chats);
  }
});
