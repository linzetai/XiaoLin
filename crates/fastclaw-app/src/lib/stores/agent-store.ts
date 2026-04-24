import type { Agent, AgentState } from "./types";
import { createChat, DEFAULT_AGENT_ID, INITIAL_AGENTS } from "./chat-helpers";
import { _persisted } from "./persistence";

type SetGet = {
  set: (partial: AgentState | Partial<AgentState> | ((s: AgentState) => AgentState | Partial<AgentState>)) => void;
  get: () => AgentState;
};

const initialActiveAgentId =
  _persisted?.activeAgentId === "default"
    ? DEFAULT_AGENT_ID
    : (_persisted?.activeAgentId ?? DEFAULT_AGENT_ID);

export function buildAgentSlice({ set, get }: SetGet) {
  return {
    agents: INITIAL_AGENTS as Agent[],
    activeAgentId: initialActiveAgentId,

    setActiveAgent: (id: string) => {
      set({ activeAgentId: id });
      get().clearUnread(id);
    },

    syncAgentsFromBackend: (backendAgents: Array<{ agentId: string; name: string; model: string }>) => {
      const COLORS = ["var(--tint)", "#34c759", "#ff9500", "#ff3b30", "#af52de", "#5856d6", "#007aff"];
      set((state) => {
        const merged: Agent[] = backendAgents.map((ba, i) => {
          const existing = state.agents.find((a) => a.id === ba.agentId);
          if (existing) return { ...existing, name: ba.name, model: ba.model, online: true };
          return {
            id: ba.agentId,
            name: ba.name,
            initial: ba.name.charAt(0).toUpperCase(),
            color: COLORS[i % COLORS.length],
            tagline: "",
            online: true,
            model: ba.model,
          };
        });

        const newChats = { ...state.agentChats };
        for (const a of merged) {
          if (!newChats[a.id]) {
            const chat = createChat();
            newChats[a.id] = { chatList: [chat], activeChatId: chat.id, unread: 0, lastMsg: null, lastTime: null };
          }
        }

        return {
          agents: merged,
          activeAgentId: merged.length > 0 && !merged.find((a) => a.id === state.activeAgentId)
            ? merged[0].id
            : state.activeAgentId,
          agentChats: newChats,
        };
      });
    },

    removeAgent: (agentId: string) => {
      set((state) => {
        const filtered = state.agents.filter((a) => a.id !== agentId);
        if (filtered.length === 0) return state;
        const { [agentId]: _, ...remainChats } = state.agentChats;
        const newActive = state.activeAgentId === agentId ? filtered[0].id : state.activeAgentId;
        return { agents: filtered, activeAgentId: newActive, agentChats: remainChats };
      });
    },
  };
}
