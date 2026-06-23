import { create } from "zustand";
import { isTauri } from "../transport";
import { fillChatFromBrowserSelection } from "./composer-input-store";

export const MAX_BROWSER_PAGES = 8;
export const MAX_BROWSER_DOWNLOADS = 50;
export const MIN_CHAT_PANEL_WIDTH = 280;
export const MAX_CHAT_PANEL_WIDTH = 500;
export const DEFAULT_CHAT_PANEL_WIDTH = 360;
export const COLLAPSED_CHAT_PANEL_WIDTH = 48;

const CHAT_PANEL_WIDTH_KEY = "xiaolin:browser-chat-panel-width";

export type PageLoadState =
  | { state: "loading" }
  | { state: "ready" }
  | { state: "failed"; message: string };

export interface BrowserPage {
  pageId: string;
  url: string;
  title: string;
  visibility: "active" | "hidden";
  loadState: PageLoadState;
  agentControlled: boolean;
}

export interface BrowserDownload {
  id: string;
  pageId: string;
  url: string;
  filename: string;
  path: string | null;
  status: "downloading" | "finished" | "failed";
}

export interface AgentOperation {
  id: string;
  pageId: string;
  action: string;
  description: string;
  ts: number;
}

interface BackendPageInfo {
  pageId: string;
  url: string;
  title: string;
  visibility: "active" | "hidden";
  loadState: PageLoadState | { state: string; message?: string; "0"?: string };
}

async function browserInvoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (!isTauri) throw new Error("Browser features require Tauri");
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<T>(cmd, args);
}

function loadChatPanelWidth(): number {
  try {
    const raw = localStorage.getItem(CHAT_PANEL_WIDTH_KEY);
    if (!raw) return DEFAULT_CHAT_PANEL_WIDTH;
    const n = Number(raw);
    if (Number.isFinite(n) && n >= MIN_CHAT_PANEL_WIDTH && n <= MAX_CHAT_PANEL_WIDTH) return n;
  } catch {
    /* ignore */
  }
  return DEFAULT_CHAT_PANEL_WIDTH;
}

function normalizeLoadState(raw: BackendPageInfo["loadState"]): PageLoadState {
  if (!raw || typeof raw !== "object") return { state: "loading" };
  const state = raw.state;
  if (state === "ready") return { state: "ready" };
  if (state === "loading") return { state: "loading" };
  if (state === "failed") {
    const msg =
      "message" in raw && typeof raw.message === "string"
        ? raw.message
        : "0" in raw && typeof raw["0"] === "string"
          ? raw["0"]
          : "load failed";
    return { state: "failed", message: msg };
  }
  return { state: "loading" };
}

function pageFromBackend(info: BackendPageInfo): BrowserPage {
  return {
    pageId: info.pageId,
    url: info.url,
    title: info.title,
    visibility: info.visibility,
    loadState: normalizeLoadState(info.loadState),
    agentControlled: false,
  };
}

function downloadFilenameFromPath(path: string): string {
  const parts = path.split(/[/\\]/);
  return parts[parts.length - 1] || "download";
}

export interface BrowserState {
  pages: Record<string, BrowserPage>;
  activePageId: string | null;
  layoutMode: "panel" | "fullwidth";
  chatPanelWidth: number;
  chatPanelCollapsed: boolean;
  layoutTransitioning: boolean;
  downloads: BrowserDownload[];
  userActionToast: string | null;
  userTakeoverActive: boolean;
  agentOperations: AgentOperation[];

  openPage: (url: string) => Promise<string | null>;
  closePage: (pageId: string) => Promise<void>;
  navigate: (pageId: string, url: string) => Promise<void>;
  setActivePageId: (pageId: string) => Promise<void>;
  setLayoutMode: (mode: "panel" | "fullwidth") => Promise<void>;
  toggleChatPanel: () => void;
  setChatPanelWidth: (width: number) => void;
  setAgentControlled: (pageId: string, controlled: boolean) => void;
  dismissDownload: (id: string) => void;
  clearUserActionToast: () => void;
  clearAgentOperations: () => void;
  syncFromBackend: () => Promise<void>;
  hideAllPages: () => Promise<void>;
  showActivePage: () => Promise<void>;
}

export const useBrowserStore = create<BrowserState>((set, get) => ({
  pages: {},
  activePageId: null,
  layoutMode: "panel",
  chatPanelWidth: loadChatPanelWidth(),
  chatPanelCollapsed: false,
  layoutTransitioning: false,
  downloads: [],
  userActionToast: null,
  userTakeoverActive: false,
  agentOperations: [],

  openPage: async (url) => {
    if (!isTauri) {
      console.warn("[browser] openPage requires Tauri");
      return null;
    }
    const pageCount = Object.keys(get().pages).length;
    if (pageCount >= MAX_BROWSER_PAGES) {
      console.warn(`[browser] page limit reached (${MAX_BROWSER_PAGES})`);
      return null;
    }
    try {
      const pageId = await browserInvoke<string>("browser_open_page", { url });
      set((s) => ({
        pages: {
          ...s.pages,
          [pageId]: {
            pageId,
            url,
            title: "",
            visibility: "hidden",
            loadState: { state: "loading" },
            agentControlled: false,
          },
        },
        activePageId: pageId,
      }));
      return pageId;
    } catch (e) {
      console.warn("[browser] openPage failed:", e);
      return null;
    }
  },

  closePage: async (pageId) => {
    if (!isTauri) return;
    try {
      await browserInvoke("browser_close_page", { pageId });
    } catch (e) {
      console.warn("[browser] closePage failed:", e);
      return;
    }
    set((s) => {
      const { [pageId]: _, ...rest } = s.pages;
      const ids = Object.keys(rest);
      const nextActive =
        s.activePageId === pageId ? (ids[0] ?? null) : s.activePageId;
      return { pages: rest, activePageId: nextActive };
    });
    await get().showActivePage();
  },

  navigate: async (pageId, url) => {
    if (!isTauri) return;
    const prevUrl = get().pages[pageId]?.url;
    set((s) => {
      const page = s.pages[pageId];
      if (!page) return s;
      return {
        pages: {
          ...s.pages,
          [pageId]: { ...page, url, loadState: { state: "loading" } },
        },
      };
    });
    try {
      await browserInvoke("browser_navigate", { pageId, url });
    } catch (e) {
      console.warn("[browser] navigate failed:", e);
      set((s) => {
        const page = s.pages[pageId];
        if (!page) return s;
        return {
          pages: {
            ...s.pages,
            [pageId]: {
              ...page,
              url: prevUrl ?? page.url,
              loadState: { state: "failed", message: "导航失败" },
            },
          },
        };
      });
    }
  },

  setActivePageId: async (pageId) => {
    if (!get().pages[pageId]) return;
    set({ activePageId: pageId });
    try {
      await browserInvoke("browser_show_page", { pageId });
    } catch (e) {
      console.warn("[browser] show_page failed:", e);
    }
  },

  setLayoutMode: async (mode) => {
    const prev = get().layoutMode;
    if (prev === mode) return;
    set({ layoutTransitioning: true });
    try {
      await browserInvoke("browser_hide_all_pages");
    } catch (e) {
      console.warn("[browser] hide during layout switch failed:", e);
    }
    set({ layoutMode: mode });
    window.setTimeout(async () => {
      set({ layoutTransitioning: false });
      await get().showActivePage();
    }, 400);
  },

  toggleChatPanel: () => set((s) => ({ chatPanelCollapsed: !s.chatPanelCollapsed })),

  setChatPanelWidth: (width) => {
    const clamped = Math.round(
      Math.min(MAX_CHAT_PANEL_WIDTH, Math.max(MIN_CHAT_PANEL_WIDTH, width)),
    );
    try {
      localStorage.setItem(CHAT_PANEL_WIDTH_KEY, String(clamped));
    } catch {
      /* ignore */
    }
    set({ chatPanelWidth: clamped });
  },

  setAgentControlled: (pageId, controlled) => {
    set((s) => {
      const page = s.pages[pageId];
      if (!page) return s;
      return {
        pages: { ...s.pages, [pageId]: { ...page, agentControlled: controlled } },
      };
    });
  },

  dismissDownload: (id) => {
    set((s) => ({ downloads: s.downloads.filter((d) => d.id !== id) }));
  },

  clearUserActionToast: () => set({ userActionToast: null }),

  clearAgentOperations: () => set({ agentOperations: [] }),

  syncFromBackend: async () => {
    if (!isTauri) return;
    try {
      const list = await browserInvoke<BackendPageInfo[]>("browser_list_pages");
      const pages: Record<string, BrowserPage> = {};
      let activePageId: string | null = null;
      for (const info of list) {
        pages[info.pageId] = pageFromBackend(info);
        if (info.visibility === "active") activePageId = info.pageId;
      }
      if (!activePageId) {
        activePageId = list[0]?.pageId ?? null;
      }
      set({ pages, activePageId });
    } catch (e) {
      console.warn("[browser] syncFromBackend failed:", e);
    }
  },

  hideAllPages: async () => {
    if (!isTauri) return;
    try {
      await browserInvoke("browser_hide_all_pages");
    } catch (e) {
      console.warn("[browser] hideAllPages failed:", e);
    }
  },

  showActivePage: async () => {
    const { activePageId } = get();
    if (!activePageId || !isTauri) return;
    try {
      await browserInvoke("browser_show_page", { pageId: activePageId });
    } catch (e) {
      console.warn("[browser] showActivePage failed:", e);
    }
  },
}));

export function hasBrowserPages(): boolean {
  return Object.keys(useBrowserStore.getState().pages).length > 0;
}

export function shouldShowBrowserWebView(opts: {
  layoutMode: "panel" | "fullwidth";
  panelOpen: boolean;
  activeTabId: string | null;
}): boolean {
  if (!hasBrowserPages()) return false;
  if (opts.layoutMode === "fullwidth") return true;
  return opts.panelOpen && opts.activeTabId === "browser";
}

const MAX_AGENT_OPERATIONS = 100;
const eventUnlisteners: Array<() => void> = [];

function pushAgentOperation(pageId: string, action: string, description: string, ts?: number) {
  const op: AgentOperation = {
    id: `${pageId}-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`,
    pageId,
    action,
    description,
    ts: ts ?? Date.now(),
  };
  useBrowserStore.setState((s) => ({
    agentOperations: [...s.agentOperations, op].slice(-MAX_AGENT_OPERATIONS),
  }));
}

function updatePage(pageId: string, patch: Partial<BrowserPage>) {
  useBrowserStore.setState((s) => {
    const page = s.pages[pageId];
    if (!page) return s;
    return { pages: { ...s.pages, [pageId]: { ...page, ...patch } } };
  });
}

export async function initBrowserEvents(): Promise<void> {
  if (!isTauri) return;
  teardownBrowserEvents();
  await useBrowserStore.getState().syncFromBackend();

  const { listen } = await import("@tauri-apps/api/event");

  eventUnlisteners.push(
    await listen<{ pageId: string; url: string }>("browser-page-created", (ev) => {
      const { pageId, url } = ev.payload;
      const existing = useBrowserStore.getState().pages[pageId];
      if (existing) return;
      useBrowserStore.setState((s) => ({
        pages: {
          ...s.pages,
          [pageId]: {
            pageId,
            url,
            title: "",
            visibility: "hidden",
            loadState: { state: "loading" },
            agentControlled: false,
          },
        },
        activePageId: pageId,
      }));
      void useBrowserStore.getState().showActivePage();
    }),
  );

  eventUnlisteners.push(
    await listen<{ pageId: string }>("browser-page-closed", (ev) => {
      const { pageId } = ev.payload;
      useBrowserStore.setState((s) => {
        const { [pageId]: _, ...rest } = s.pages;
        const ids = Object.keys(rest);
        const nextActive = s.activePageId === pageId ? (ids[0] ?? null) : s.activePageId;
        return { pages: rest, activePageId: nextActive };
      });
      void useBrowserStore.getState().showActivePage();
    }),
  );

  eventUnlisteners.push(
    await listen<{ pageId: string; url: string }>("browser-url-changed", (ev) => {
      const { pageId, url } = ev.payload;
      updatePage(pageId, { url });
    }),
  );

  eventUnlisteners.push(
    await listen<{ pageId: string; title: string }>("browser-title-changed", (ev) => {
      const { pageId, title } = ev.payload;
      updatePage(pageId, { title });
    }),
  );

  eventUnlisteners.push(
    await listen<{
      pageId: string;
      loading?: boolean;
      loadState?: BackendPageInfo["loadState"];
      url?: string;
    }>("browser-loading", (ev) => {
      const { pageId, loadState, url } = ev.payload;
      const patch: Partial<BrowserPage> = {};
      if (loadState) patch.loadState = normalizeLoadState(loadState);
      if (url) patch.url = url;
      updatePage(pageId, patch);
    }),
  );

  eventUnlisteners.push(
    await listen<{ pageId: string; type?: string; data?: unknown; ts?: number }>(
      "browser-user-action",
      (ev) => {
        const { pageId, type, data, ts } = ev.payload;

        if (type === "agent_op" && data && typeof data === "object") {
          const { action, description } = data as { action?: string; description?: string };
          if (action) {
            pushAgentOperation(pageId, action, description ?? action, ts);
          }
          return;
        }

        if (type === "user_action_blocked") {
          useBrowserStore.setState({
            userActionToast: "Agent 操作中，用户输入已拦截",
          });
          window.setTimeout(() => {
            useBrowserStore.getState().clearUserActionToast();
          }, 2500);
          return;
        }

        if (type === "selection" && data && typeof data === "object") {
          const { action, text, url } = data as {
            action?: string;
            text?: string;
            url?: string;
          };
          if (action === "copy") {
            return;
          }
          if (text && (action === "ask" || action === "quote")) {
            const page = useBrowserStore.getState().pages[pageId];
            fillChatFromBrowserSelection({
              action,
              text,
              url: url ?? page?.url ?? "",
            });
            useBrowserStore.setState({
              userActionToast:
                action === "ask" ? "已填入 Chat 输入框" : "已追加引用到 Chat",
            });
            window.setTimeout(() => {
              useBrowserStore.getState().clearUserActionToast();
            }, 2500);
          }
        }

        console.info("[browser] user action", pageId, type, data);
      },
    ),
  );

  eventUnlisteners.push(
    await listen<{ pageId: string; active: boolean }>("browser-agent-control", (ev) => {
      const { pageId, active } = ev.payload;
      useBrowserStore.getState().setAgentControlled(pageId, active);
    }),
  );

  eventUnlisteners.push(
    await listen<{ pageId: string; url: string; destination: string }>(
      "browser-download-requested",
      (ev) => {
        const { pageId, url, destination } = ev.payload;
        const id = `${pageId}-${Date.now()}`;
        const download: BrowserDownload = {
          id,
          pageId,
          url,
          filename: downloadFilenameFromPath(destination),
          path: destination,
          status: "downloading",
        };
        useBrowserStore.setState((s) => ({
          downloads: [...s.downloads, download].slice(-MAX_BROWSER_DOWNLOADS),
        }));
      },
    ),
  );

  eventUnlisteners.push(
    await listen<{ pageId: string; url: string; path?: string; success?: boolean }>(
      "browser-download-finished",
      (ev) => {
        const { pageId, url, path, success } = ev.payload;
        // Backend does not emit downloadId; match pageId+url+status and filename when available.
        const finishedFilename = path ? downloadFilenameFromPath(path) : null;
        useBrowserStore.setState((s) => ({
          downloads: s.downloads.map((d) => {
            if (d.pageId !== pageId || d.url !== url || d.status !== "downloading") return d;
            if (finishedFilename && d.filename !== finishedFilename) return d;
            return {
              ...d,
              path: path ?? d.path,
              status: success ? "finished" : "failed",
            };
          }),
        }));
      },
    ),
  );
}

export function teardownBrowserEvents(): void {
  while (eventUnlisteners.length > 0) {
    const unlisten = eventUnlisteners.pop();
    unlisten?.();
  }
}

export async function browserGoBack(pageId: string): Promise<void> {
  if (!isTauri) return;
  try {
    await browserInvoke("browser_go_back", { pageId });
  } catch (e) {
    console.warn("[browser] goBack failed:", e);
  }
}

export async function browserGoForward(pageId: string): Promise<void> {
  if (!isTauri) return;
  try {
    await browserInvoke("browser_go_forward", { pageId });
  } catch (e) {
    console.warn("[browser] goForward failed:", e);
  }
}

export async function browserReload(pageId: string): Promise<void> {
  if (!isTauri) return;
  try {
    await browserInvoke("browser_reload", { pageId });
  } catch (e) {
    console.warn("[browser] reload failed:", e);
  }
}

export async function browserResizeWebview(
  pageId: string,
  x: number,
  y: number,
  width: number,
  height: number,
  scaleFactor?: number,
): Promise<void> {
  if (!isTauri) return;
  try {
    await browserInvoke("browser_resize_webview", { pageId, x, y, width, height, scaleFactor });
  } catch (e) {
    console.warn("[browser] resize failed:", e);
  }
}

export function normalizeNavUrl(input: string): string | null {
  const trimmed = input.trim();
  if (!trimmed) return null;
  try {
    const parsed = trimmed.includes("://") ? new URL(trimmed) : new URL(`https://${trimmed}`);
    if (parsed.protocol !== "http:" && parsed.protocol !== "https:") return null;
    return parsed.toString();
  } catch {
    return null;
  }
}

export function isHttpsUrl(url: string): boolean {
  try {
    return new URL(url).protocol === "https:";
  } catch {
    return false;
  }
}

/** Request user takeover — stops agent browser actions and restores user control. */
export async function browserRequestTakeover(pageId: string): Promise<void> {
  if (!isTauri) return;
  try {
    await browserInvoke("browser_request_takeover", { pageId });
  } catch (e) {
    console.warn("[browser] requestTakeover failed:", e);
    return;
  }
  useBrowserStore.getState().setAgentControlled(pageId, false);
  useBrowserStore.setState({ userTakeoverActive: true });
}

/** Clear user takeover flag so the agent may resume browser actions. */
export async function browserClearUserTakeover(): Promise<void> {
  if (!isTauri) return;
  try {
    await browserInvoke("browser_clear_user_takeover");
    useBrowserStore.setState({ userTakeoverActive: false });
  } catch (e) {
    console.warn("[browser] clearUserTakeover failed:", e);
  }
}
