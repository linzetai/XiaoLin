import { create } from "zustand";
import * as transport from "../transport";
import type { PluginSummary, PluginTool } from "../transport";

export interface PluginStoreState {
  plugins: PluginSummary[];
  loading: boolean;
  error: string | null;
  toolsById: Record<string, PluginTool[]>;

  fetchPlugins: () => Promise<void>;
  enablePlugin: (id: string) => Promise<boolean>;
  disablePlugin: (id: string) => Promise<boolean>;
  restartPlugin: (id: string) => Promise<boolean>;
  approvePlugin: (id: string) => Promise<boolean>;
  rejectPlugin: (id: string) => Promise<boolean>;
  fetchTools: (id: string) => Promise<PluginTool[]>;
  connectedCount: () => number;
}

export const usePluginStore = create<PluginStoreState>((set, get) => ({
  plugins: [],
  loading: false,
  error: null,
  toolsById: {},

  fetchPlugins: async () => {
    set({ loading: true, error: null });
    try {
      const plugins = await transport.listPlugins();
      set({ plugins, loading: false });
    } catch (e) {
      set({ error: String(e), loading: false });
    }
  },

  enablePlugin: async (id) => {
    try {
      const ok = await transport.enablePlugin(id);
      if (ok) await get().fetchPlugins();
      return ok;
    } catch (e) {
      set({ error: String(e) });
      return false;
    }
  },

  disablePlugin: async (id) => {
    try {
      const ok = await transport.disablePlugin(id);
      if (ok) await get().fetchPlugins();
      return ok;
    } catch (e) {
      set({ error: String(e) });
      return false;
    }
  },

  restartPlugin: async (id) => {
    try {
      const ok = await transport.restartPlugin(id);
      if (ok) await get().fetchPlugins();
      return ok;
    } catch (e) {
      set({ error: String(e) });
      return false;
    }
  },

  approvePlugin: async (id) => {
    try {
      const ok = await transport.approvePlugin(id);
      if (ok) await get().fetchPlugins();
      return ok;
    } catch (e) {
      set({ error: String(e) });
      return false;
    }
  },

  rejectPlugin: async (id) => {
    try {
      const ok = await transport.rejectPlugin(id);
      if (ok) await get().fetchPlugins();
      return ok;
    } catch (e) {
      set({ error: String(e) });
      return false;
    }
  },

  fetchTools: async (id) => {
    try {
      const tools = await transport.getPluginTools(id);
      set((s) => ({ toolsById: { ...s.toolsById, [id]: tools } }));
      return tools;
    } catch (e) {
      set({ error: String(e) });
      return [];
    }
  },

  connectedCount: () => {
    return get().plugins.filter((p) => p.status === "connected").length;
  },
}));

let unsubscribe: (() => void) | null = null;

export function subscribePluginEvents() {
  if (unsubscribe) return;
  unsubscribe = transport.onPluginsStatusChanged((plugins) => {
    usePluginStore.setState({ plugins });
  });
}

export function unsubscribePluginEvents() {
  unsubscribe?.();
  unsubscribe = null;
}
