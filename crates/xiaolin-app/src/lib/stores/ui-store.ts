import { create } from "zustand";

export type NavItem = "chat" | "workspace" | "tasks" | "files" | "connections";

export interface UIState {
  detailOpen: boolean;
  sidebarCollapsed: boolean;
  activeNav: NavItem;

  toggleDetail: () => void;
  closeDetail: () => void;
  toggleSidebar: () => void;
  setActiveNav: (nav: NavItem) => void;
}

export const useUIStore = create<UIState>((set) => ({
  detailOpen: false,
  sidebarCollapsed: false,
  activeNav: "chat" as NavItem,

  toggleDetail: () => set((s) => ({ detailOpen: !s.detailOpen })),
  closeDetail: () => set({ detailOpen: false }),
  toggleSidebar: () => set((s) => ({ sidebarCollapsed: !s.sidebarCollapsed })),
  setActiveNav: (nav) => set({ activeNav: nav }),
}));

export function buildUISlice(set: (partial: Partial<{ detailOpen: boolean; sidebarCollapsed: boolean; activeNav: NavItem }>) => void) {
  return {
    detailOpen: false,
    sidebarCollapsed: false,
    activeNav: "chat" as NavItem,

    toggleDetail: () => set({ detailOpen: !useUIStore.getState().detailOpen }),
    closeDetail: () => set({ detailOpen: false }),
    toggleSidebar: () => set({ sidebarCollapsed: !useUIStore.getState().sidebarCollapsed }),
    setActiveNav: (nav: NavItem) => set({ activeNav: nav }),
  };
}
