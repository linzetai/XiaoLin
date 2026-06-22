import { create } from "zustand";
import type { ComponentType } from "react";

export interface WorkspaceTab {
  id: string;
  label: string;
  icon: ComponentType<{ size?: number; strokeWidth?: number }>;
  component: ComponentType;
  footerComponent?: ComponentType;
  badge?: number | boolean;
  order?: number;
}

const isTauri =
  typeof window !== "undefined" &&
  ("__TAURI_INTERNALS__" in window || "__TAURI__" in window);

const DEFAULT_PANEL_WIDTH = 360;
const MIN_PANEL_WIDTH = 260;
const MAX_PANEL_WIDTH = 700;

async function resizeWindowForPanel(opening: boolean, prePanelWidth: number | null, panelWidth: number): Promise<number | null> {
  if (!isTauri) return null;

  try {
    const { getCurrentWindow } = await import("@tauri-apps/api/window");
    const { currentMonitor } = await import("@tauri-apps/api/window");
    const win = getCurrentWindow();

    if (await win.isMaximized()) return null;

    const size = await win.innerSize();
    const pos = await win.outerPosition();
    const monitor = await currentMonitor();

    if (opening) {
      if (monitor) {
        const availableRight = monitor.position.x + monitor.size.width;
        const windowRight = pos.x + size.width + panelWidth;
        if (windowRight > availableRight) return null;
      }
      const savedWidth = size.width;
      await win.setSize(new (await import("@tauri-apps/api/dpi")).LogicalSize(
        size.toLogical((await win.scaleFactor())).width + panelWidth,
        size.toLogical((await win.scaleFactor())).height,
      ));
      return savedWidth;
    } else {
      if (prePanelWidth != null) {
        const scale = await win.scaleFactor();
        const logicalSize = size.toLogical(scale);
        await win.setSize(new (await import("@tauri-apps/api/dpi")).LogicalSize(
          logicalSize.width - panelWidth,
          logicalSize.height,
        ));
      }
      return null;
    }
  } catch {
    return null;
  }
}

interface WorkspaceTabsState {
  tabs: WorkspaceTab[];
  activeTabId: string | null;
  panelOpen: boolean;
  panelWidth: number;
  prePanelWidth: number | null;
  /** Per-session tab memory: sessionId → last activeTabId */
  sessionTabMap: Record<string, string>;
  /** True when user manually closed the Plan tab — suppresses auto-open until session switch */
  planClosedByUser: boolean;
  /** True when user manually closed the Files tab — suppresses auto-open until session switch */
  filesClosedByUser: boolean;

  registerTab: (tab: WorkspaceTab) => void;
  unregisterTab: (id: string) => void;
  setActiveTab: (id: string) => void;
  setPanelOpen: (open: boolean) => void;
  togglePanel: () => void;
  setPanelWidth: (width: number) => void;
  setPlanClosedByUser: (closed: boolean) => void;
  setFilesClosedByUser: (closed: boolean) => void;
  /** Call when active session changes to save/restore tab state */
  switchSession: (newSessionId: string, oldSessionId?: string) => void;
}

function loadPanelWidth(): number {
  try {
    const saved = localStorage.getItem("xiaolin:panel-width");
    if (saved) {
      const n = Number(saved);
      if (n >= MIN_PANEL_WIDTH && n <= MAX_PANEL_WIDTH) return n;
    }
  } catch {}
  return DEFAULT_PANEL_WIDTH;
}

export const useWorkspaceTabs = create<WorkspaceTabsState>((set, get) => ({
  tabs: [],
  activeTabId: null,
  panelOpen: false,
  panelWidth: loadPanelWidth(),
  prePanelWidth: null,
  sessionTabMap: {},
  planClosedByUser: false,
  filesClosedByUser: false,

  registerTab: (tab) => {
    set((s) => {
      if (s.tabs.some((t) => t.id === tab.id)) return s;
      const tabs = [...s.tabs, tab].sort((a, b) => (a.order ?? 99) - (b.order ?? 99));
      return { tabs, activeTabId: s.activeTabId ?? tab.id };
    });
  },

  unregisterTab: (id) => {
    set((s) => {
      const tabs = s.tabs.filter((t) => t.id !== id);
      const activeTabId =
        s.activeTabId === id ? (tabs[0]?.id ?? null) : s.activeTabId;
      return { tabs, activeTabId };
    });
  },

  setActiveTab: (id) => {
    const { tabs, panelOpen, panelWidth } = get();
    if (tabs.some((t) => t.id === id)) {
      if (!panelOpen) {
        set({ activeTabId: id, panelOpen: true });
        resizeWindowForPanel(true, null, panelWidth).then((saved) => {
          if (saved != null) set({ prePanelWidth: saved });
        });
      } else {
        set({ activeTabId: id });
      }
    }
  },

  setPanelOpen: (open) => {
    const { panelOpen, prePanelWidth, panelWidth } = get();
    if (open === panelOpen) return;
    set({ panelOpen: open });
    if (open) {
      resizeWindowForPanel(true, null, panelWidth).then((saved) => {
        if (saved != null) set({ prePanelWidth: saved });
      });
    } else {
      resizeWindowForPanel(false, prePanelWidth, panelWidth).then(() => {
        set({ prePanelWidth: null });
      });
    }
  },

  togglePanel: () => {
    const { panelOpen, prePanelWidth, panelWidth } = get();
    const next = !panelOpen;
    set({ panelOpen: next });
    if (next) {
      resizeWindowForPanel(true, null, panelWidth).then((saved) => {
        if (saved != null) set({ prePanelWidth: saved });
      });
    } else {
      resizeWindowForPanel(false, prePanelWidth, panelWidth).then(() => {
        set({ prePanelWidth: null });
      });
    }
  },

  setPlanClosedByUser: (closed) => set({ planClosedByUser: closed }),

  setFilesClosedByUser: (closed) => set({ filesClosedByUser: closed }),

  setPanelWidth: (width) => {
    const clamped = Math.max(MIN_PANEL_WIDTH, Math.min(MAX_PANEL_WIDTH, width));
    set({ panelWidth: clamped });
    try {
      localStorage.setItem("xiaolin:panel-width", String(clamped));
    } catch {}
  },

  switchSession: (newSessionId, oldSessionId) => {
    const { activeTabId, tabs, sessionTabMap } = get();
    const updates: Partial<WorkspaceTabsState> = {
      planClosedByUser: false,
      filesClosedByUser: false,
    };

    if (oldSessionId && activeTabId) {
      updates.sessionTabMap = { ...sessionTabMap, [oldSessionId]: activeTabId };
    }

    const savedTab = (updates.sessionTabMap ?? sessionTabMap)[newSessionId];
    if (savedTab && tabs.some((t) => t.id === savedTab)) {
      updates.activeTabId = savedTab;
    }

    set(updates as WorkspaceTabsState);
  },
}));
