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
  mode: "embedded" | "remote" | "browser" | "connecting";
  info: GatewayInfo | null;
  connected: boolean;
  error: string | null;

  init: () => Promise<void>;
  setConnected: (v: boolean) => void;
}

let disconnectUnsub: (() => void) | null = null;
let reconnectedUnsub: (() => void) | null = null;
let sessionChangedUnsub: (() => void) | null = null;

async function syncBackendData() {
  try {
    const [agents, sessions] = await Promise.all([
      transport.listAgents(),
      transport.listSessions(50),
    ]);
    if (agents.length > 0) {
      useAgentStore.getState().syncAgentsFromBackend(agents);
      for (const agent of agents) {
        const agentSessions = sessions.filter(
          (s) => s.agentId === agent.agentId,
        );
        if (agentSessions.length > 0) {
          useAgentStore
            .getState()
            .syncSessionsForAgent(agent.agentId, agentSessions);
        }
      }
    }
  } catch {
    /* sync failure is non-fatal */
  }
}

export const useGatewayStore = create<GatewayState>((set) => ({
  mode: "connecting",
  info: null,
  connected: false,
  error: null,

  init: async () => {
    try {
      disconnectUnsub?.();
      reconnectedUnsub?.();
      sessionChangedUnsub?.();

      set({ mode: "connecting", error: null });

      if (transport.isTauri) {
        // Tauri mode: use IPC commands directly, no WebSocket needed
        const info = await transport.invokeWithRetry<GatewayInfo>(
          "get_gateway_info",
        );
        set({ mode: "embedded", info, connected: true, error: null });

        sessionChangedUnsub = await transport.onSessionChanged(
          async (sid) => {
            try {
              const session = await transport.getSession(sid);
              if (session?.title) {
                const store = useAgentStore.getState();
                const agentId = session.agentId || store.activeAgentId;
                store.renameChat(agentId, sid, session.title);
              }
            } catch {
              /* ignore */
            }
          },
        );

        await syncBackendData();
      } else {
        // Browser mode: use WebSocket + HTTP
        const info = await fetchBrowserGatewayInfo();
        const mode = info.wsUrl ? "remote" : "browser";
        set({ mode, info, error: null });

        if (info.wsUrl) {
          disconnectUnsub = transport.onWsEvent("disconnected", () => {
            set({ connected: false });
          });

          reconnectedUnsub = transport.onWsEvent("reconnected", () => {
            set({ connected: true });
            syncBackendData();
          });

          try {
            await transport.connectWs(info.wsUrl);
            set({ connected: true });

            sessionChangedUnsub = await transport.onSessionChanged(
              async (sid) => {
                try {
                  const session = await transport.getSession(sid);
                  if (session?.title) {
                    const store = useAgentStore.getState();
                    const agentId = session.agentId || store.activeAgentId;
                    store.renameChat(agentId, sid, session.title);
                  }
                } catch {
                  /* ignore */
                }
              },
            );

            await syncBackendData();
          } catch (e) {
            console.warn("WS connect failed:", e);
            set({ connected: false });
          }
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
