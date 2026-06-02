import type { Agent, AgentState } from "./types";
import { DEFAULT_AGENT_ID, INITIAL_AGENTS } from "./chat-helpers";

type SetGet = {
  set: (partial: AgentState | Partial<AgentState> | ((s: AgentState) => AgentState | Partial<AgentState>)) => void;
  get: () => AgentState;
};

export function buildAgentSlice({ set }: SetGet) {
  return {
    agents: INITIAL_AGENTS as Agent[],
    activeAgentId: DEFAULT_AGENT_ID,

    setActiveAgent: (_id: string) => {
      // No-op in single-agent mode — activeAgentId is always "main"
    },

    syncAgentsFromBackend: (backendAgents: Array<{ agentId: string; name: string; model: string; avatar?: string | null }>) => {
      const main = backendAgents.find((a) => a.agentId === DEFAULT_AGENT_ID);
      if (!main) return;
      set((state) => {
        const agents = state.agents.map((a) => {
          if (a.id !== DEFAULT_AGENT_ID) return a;
          return { ...a, model: main.model || a.model, name: main.name || a.name };
        });
        return { agents };
      });
    },

    updateAgentProps: (_agentId: string, props: Partial<Pick<Agent, "name" | "model" | "avatar">>) => {
      set((state) => {
        const agents = state.agents.map((a) => {
          if (a.id !== DEFAULT_AGENT_ID) return a;
          const updated = { ...a, ...props };
          if (props.name) updated.initial = props.name.charAt(0).toUpperCase();
          return updated;
        });
        return { agents };
      });
    },

    removeAgent: (_agentId: string) => {
      // No-op in single-agent mode
    },
  };
}
