import { create } from "zustand";
import * as transport from "../transport";
import { useChatMetaStore } from "./chat-meta-store";
import { useUIStore } from "./ui-store";

export interface SearchResult {
  session_id: string;
  turn_id: string;
  role: string;
  message_id: string | null;
  session_title: string;
  work_dir: string | null;
  snippet: string;
  timestamp: string;
  rank: number;
}

export interface SearchFilters {
  work_dir?: string;
  date_from?: string;
  date_to?: string;
}

export interface SearchIndexStatus {
  indexed_count: number;
  total_count: number;
  is_indexing: boolean;
}

const PAGE_LIMIT = 10;
const DEBOUNCE_MS = 300;
const INDEX_POLL_MS = 5000;
const NAV_SCROLL_TIMEOUT_MS = 3000;

let debounceTimer: ReturnType<typeof setTimeout> | null = null;
let searchRequestId = 0;
let indexPollTimer: ReturnType<typeof setInterval> | null = null;

function stopIndexPolling() {
  if (indexPollTimer !== null) {
    clearInterval(indexPollTimer);
    indexPollTimer = null;
  }
}

function startIndexPolling(fetchIndexStatus: () => Promise<void>) {
  stopIndexPolling();
  indexPollTimer = setInterval(() => {
    void fetchIndexStatus();
  }, INDEX_POLL_MS);
}

export interface SearchState {
  panelOpen: boolean;
  query: string;
  results: SearchResult[];
  loading: boolean;
  filters: SearchFilters;
  page: number;
  hasMore: boolean;
  indexStatus: SearchIndexStatus | null;
  pendingScrollTurnId: string | null;
  pendingScrollSessionId: string | null;
  highlightTurnId: string | null;
  navError: string | null;

  openPanel: () => void;
  closePanel: () => void;
  setQuery: (q: string) => void;
  setFilters: (f: Partial<SearchFilters>) => void;
  search: () => Promise<void>;
  loadMore: () => Promise<void>;
  navigateToResult: (result: SearchResult) => void;
  fetchIndexStatus: () => Promise<void>;
  clearPendingScroll: () => void;
  clearHighlight: () => void;
  setNavError: (msg: string | null) => void;
}

export const useSearchStore = create<SearchState>((set, get) => ({
  panelOpen: false,
  query: "",
  results: [],
  loading: false,
  filters: {},
  page: 0,
  hasMore: false,
  indexStatus: null,
  pendingScrollTurnId: null,
  pendingScrollSessionId: null,
  highlightTurnId: null,
  navError: null,

  openPanel: () => {
    set({ panelOpen: true });
    void get().fetchIndexStatus();
    const status = get().indexStatus;
    if (status?.is_indexing) {
      startIndexPolling(() => get().fetchIndexStatus());
    }
  },

  closePanel: () => {
    set({ panelOpen: false });
    stopIndexPolling();
    if (debounceTimer !== null) {
      clearTimeout(debounceTimer);
      debounceTimer = null;
    }
  },

  setQuery: (q) => {
    set({ query: q, page: 0 });
    if (debounceTimer !== null) clearTimeout(debounceTimer);
    if (!q.trim()) {
      set({ results: [], hasMore: false, loading: false });
      return;
    }
    debounceTimer = setTimeout(() => {
      debounceTimer = null;
      void get().search();
    }, DEBOUNCE_MS);
  },

  setFilters: (f) => {
    set((s) => ({ filters: { ...s.filters, ...f }, page: 0 }));
    const { query } = get();
    if (query.trim()) {
      void get().search();
    }
  },

  search: async () => {
    const { query, filters, page } = get();
    const trimmed = query.trim();
    if (!trimmed) {
      set({ results: [], hasMore: false, loading: false });
      return;
    }

    const reqId = ++searchRequestId;
    set({ loading: true });

    try {
      const resp = await transport.searchQuery({
        q: trimmed,
        filters,
        page,
        limit: PAGE_LIMIT,
      });

      if (reqId !== searchRequestId) return;

      const hasMore =
        resp.results.length >= PAGE_LIMIT ||
        (page === 0
          ? resp.results.length < resp.total_estimate
          : get().results.length + resp.results.length < resp.total_estimate);

      set({
        results: page === 0 ? resp.results : [...get().results, ...resp.results],
        hasMore,
        loading: false,
        page,
      });
    } catch (e) {
      if (reqId !== searchRequestId) return;
      console.warn("[search-store] search failed:", e);
      set({ loading: false });
    }
  },

  loadMore: async () => {
    const { hasMore, loading, page, query, filters } = get();
    if (!hasMore || loading) return;
    const nextPage = page + 1;
    const reqId = ++searchRequestId;
    set({ loading: true, page: nextPage });

    try {
      const resp = await transport.searchQuery({
        q: query.trim(),
        filters,
        page: nextPage,
        limit: PAGE_LIMIT,
      });
      if (reqId !== searchRequestId) return;

      const combined = [...get().results, ...resp.results];
      const hasMoreNext =
        resp.results.length >= PAGE_LIMIT ||
        combined.length < resp.total_estimate;

      set({ results: combined, hasMore: hasMoreNext, loading: false });
    } catch (e) {
      if (reqId !== searchRequestId) return;
      console.warn("[search-store] loadMore failed:", e);
      set({ loading: false, page });
    }
  },

  navigateToResult: (result) => {
    get().closePanel();
    useUIStore.getState().setMainView("chat");
    useChatMetaStore.getState().setActiveChat(result.session_id);
    set({
      pendingScrollTurnId: result.turn_id,
      pendingScrollSessionId: result.session_id,
      highlightTurnId: result.turn_id,
    });

    setTimeout(() => {
      const state = get();
      if (
        state.pendingScrollTurnId === result.turn_id &&
        state.pendingScrollSessionId === result.session_id
      ) {
        state.setNavError("navFailed");
        state.clearPendingScroll();
        state.clearHighlight();
      }
    }, NAV_SCROLL_TIMEOUT_MS);
  },

  fetchIndexStatus: async () => {
    try {
      const status = await transport.searchIndexStatus();
      set({ indexStatus: status });
      if (status.is_indexing && get().panelOpen) {
        if (!indexPollTimer) {
          startIndexPolling(() => get().fetchIndexStatus());
        }
      } else {
        stopIndexPolling();
      }
    } catch (e) {
      console.warn("[search-store] index status failed:", e);
    }
  },

  clearPendingScroll: () => {
    set({ pendingScrollTurnId: null, pendingScrollSessionId: null });
  },

  clearHighlight: () => {
    set({ highlightTurnId: null });
  },

  setNavError: (msg) => {
    set({ navError: msg });
    if (msg) {
      setTimeout(() => {
        if (get().navError === msg) {
          set({ navError: null });
        }
      }, 4000);
    }
  },
}));
