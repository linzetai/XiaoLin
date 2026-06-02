import { create } from "zustand";
import * as transport from "./transport";
import { useAgentStore } from "./agent-store";

export interface GatewayInfo {
  port: number;
  wsUrl: string;
  httpUrl: string;
  version: string;
}

interface GatewayState {
  /** shell → connecting → ready (Tauri) or shell → connecting → browser */
  mode: "shell" | "ready" | "connecting" | "browser";
  info: GatewayInfo | null;
  connected: boolean;
  error: string | null;

  init: () => Promise<void>;
  setConnected: (v: boolean) => void;
}

let disconnectUnsub: (() => void) | null = null;
let reconnectedUnsub: (() => void) | null = null;
let sessionChangedUnsub: (() => void) | null = null;

const SESSION_CACHE_KEY = "xiaolin:session-cache";

/** Restore cached sessions so the UI can render a skeleton sidebar immediately. */
function restoreCachedSessions() {
  try {
    const raw = localStorage.getItem(SESSION_CACHE_KEY);
    if (!raw) return;
    const sessions = JSON.parse(raw);
    if (Array.isArray(sessions) && sessions.length > 0) {
      useAgentStore.getState().syncSessionsForAgent("main", sessions);
    }
  } catch {
    /* cache miss is fine */
  }
}

async function syncBackendData() {
  try {
    const [sessions, agents] = await Promise.all([
      transport.listSessions(50),
      transport.listAgents(),
    ]);
    const store = useAgentStore.getState();
    if (agents.length > 0) {
      store.syncAgentsFromBackend(agents);
    }
    if (sessions.length > 0) {
      store.syncSessionsForAgent("main", sessions);
      try {
        localStorage.setItem(SESSION_CACHE_KEY, JSON.stringify(sessions));
      } catch {
        /* storage full or unavailable */
      }
    }
  } catch {
    /* sync failure is non-fatal */
  }
}

export const useGatewayStore = create<GatewayState>((set) => ({
  mode: "shell",
  info: null,
  connected: false,
  error: null,

  init: async () => {
    try {
      disconnectUnsub?.();
      reconnectedUnsub?.();
      sessionChangedUnsub?.();

      restoreCachedSessions();
      set({ mode: "connecting", error: null });

      if (transport.isTauri) {
        // Tauri mode: get gateway info from IPC, then connect via WebSocket
        const info = await transport.getGatewayInfo();
        if (!info) {
          throw new Error("Gateway not started");
        }

        // Always use WebSocket for communication
        disconnectUnsub = transport.onWsEvent("disconnected", () => {
          set({ connected: false });
        });
        reconnectedUnsub = transport.onWsEvent("reconnected", () => {
          set({ connected: true });
          syncBackendData();
        });

        try {
          await transport.connectWs(info.wsUrl);
          set({ mode: "ready", info, connected: true, error: null });
        } catch (e) {
          console.warn("WS connect failed:", e);
          set({ mode: "ready", info, connected: false, error: String(e) });
          return;
        }

        sessionChangedUnsub = transport.onSessionChanged(async (sid) => {
          try {
            const session = await transport.getSession(sid);
            if (session) {
              const store = useAgentStore.getState();
              if (session.title) store.renameChat("main", sid, session.title);
              if (session.workDir !== undefined) store.setWorkDir("main", sid, session.workDir ?? null);
            }
          } catch {
            /* ignore */
          }
        });

        await syncBackendData();
      } else {
        // Browser mode: check for gateway health endpoint
        const info = await fetchBrowserGatewayInfo();
        if (!info.wsUrl) {
          set({ mode: "browser", info: null, connected: false });
          return;
        }

        disconnectUnsub = transport.onWsEvent("disconnected", () => {
          set({ connected: false });
        });

        reconnectedUnsub = transport.onWsEvent("reconnected", () => {
          set({ connected: true });
          syncBackendData();
        });

        try {
          await transport.connectWs(info.wsUrl);
          set({ mode: "ready", info, connected: true });

          sessionChangedUnsub = transport.onSessionChanged(async (sid) => {
            try {
              const session = await transport.getSession(sid);
              if (session) {
                const store = useAgentStore.getState();
                if (session.title) store.renameChat("main", sid, session.title);
                if (session.workDir !== undefined) store.setWorkDir("main", sid, session.workDir ?? null);
              }
            } catch {
              /* ignore */
            }
          });

          await syncBackendData();
        } catch (e) {
          console.warn("WS connect failed:", e);
          set({ mode: "browser", info, connected: false });
        }
      }
    } catch (e) {
      set({ mode: "connecting", error: String(e) });
    }
  },

  setConnected: (v) => set({ connected: v }),
}));

async function fetchBrowserGatewayInfo(): Promise<GatewayInfo> {
  const port = 18888;
  const httpUrl = `http://127.0.0.1:${port}`;
  try {
    const resp = await fetch(`${httpUrl}/health`);
    if (resp.ok) {
      return {
        port,
        wsUrl: `ws://127.0.0.1:${port}/ws`,
        httpUrl,
        version: "dev",
      };
    }
  } catch {
    // gateway not running
  }
  return { port: 0, wsUrl: "", httpUrl: "", version: "dev-browser" };
}