import type { AgentState } from "./types";

/* Zustand setState is compatible; keep loose to match StoreApi. */
// eslint-disable-next-line @typescript-eslint/no-explicit-any
export function buildUISlice(set: any) {
  return {
    detailOpen: false,
    sidebarCollapsed: false,

    toggleDetail: () => set((s: AgentState) => ({ detailOpen: !s.detailOpen })),
    closeDetail: () => set({ detailOpen: false }),
    toggleSidebar: () => set((s: AgentState) => ({ sidebarCollapsed: !s.sidebarCollapsed })),
  };
}
