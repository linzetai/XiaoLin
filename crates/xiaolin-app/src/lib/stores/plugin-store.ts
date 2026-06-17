import { create } from "zustand";
import * as transport from "../transport";
import type { AddMcpServerParams, PluginSummary, PluginTool } from "../transport";
import { useStreamStore } from "./stream-store";

export interface PluginStoreState {
  plugins: PluginSummary[];
  loading: boolean;
  error: string | null;
  toolsById: Record<string, PluginTool[]>;

  fetchPlugins: () => Promise<void>;
  addPlugin: (params: AddMcpServerParams) => Promise<boolean>;
  removePlugin: (id: string) => Promise<boolean>;
  enablePlugin: (id: string) => Promise<boolean>;
  disablePlugin: (id: string) => Promise<boolean>;
  restartPlugin: (id: string) => Promise<boolean>;
  approvePlugin: (id: string) => Promise<boolean>;
  rejectPlugin: (id: string) => Promise<boolean>;
  oauthLoginPlugin: (id: string) => Promise<boolean>;
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

  addPlugin: async (params) => {
    try {
      const result = await transport.addMcpServer(params);
      if (result.ok) await get().fetchPlugins();
      return result.ok;
    } catch (e) {
      set({ error: String(e) });
      return false;
    }
  },

  removePlugin: async (id) => {
    try {
      const result = await transport.removeMcpServer(id);
      if (result.ok) {
        set((s) => {
          const { [id]: _, ...rest } = s.toolsById;
          return { toolsById: rest };
        });
        await get().fetchPlugins();
      }
      return result.ok;
    } catch (e) {
      set({ error: String(e) });
      return false;
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

  oauthLoginPlugin: async (id) => {
    try {
      const result = await transport.oauthLoginPlugin(id);
      if (result.ok && result.auth_url) {
        window.open(result.auth_url, "_blank");
      }
      return result.ok;
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

let unsubscribers: (() => void)[] = [];

export function subscribePluginEvents() {
  if (unsubscribers.length > 0) return;
  unsubscribers.push(
    transport.onPluginsStatusChanged((plugins) => {
      usePluginStore.setState({ plugins });
    }),
  );
  unsubscribers.push(
    transport.onPluginEvent("plugins.resources_changed", () => {
      usePluginStore.getState().fetchPlugins();
    }),
  );
  unsubscribers.push(
    transport.onPluginEvent("plugins.prompts_changed", () => {
      usePluginStore.getState().fetchPlugins();
    }),
  );
  unsubscribers.push(
    transport.onPluginEvent("plugins.tool_progress", (data) => {
      const token = data.progressToken as string | undefined;
      if (!token) return;
      const progress = data.progress as number | undefined;
      const total = data.total as number | undefined;
      const ratio = progress != null && total ? progress / total : undefined;
      useStreamStore.getState().setToolProgress(token, {
        progress: ratio,
        message: (data.message as string) || undefined,
      });
    }),
  );
}

export function unsubscribePluginEvents() {
  for (const unsub of unsubscribers) unsub();
  unsubscribers = [];
}
