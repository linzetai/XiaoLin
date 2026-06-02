import { create } from "zustand";
import { buildAgentSlice } from "./agent-store";
import { saveUIState } from "./persistence";
import { buildSessionSlice } from "./session-store";
import type { AgentState } from "./types";
import { buildUISlice } from "./ui-store";

export { useConfigStore } from "./config-store";
export { buildAgentSlice } from "./agent-store";
export { buildSessionSlice } from "./session-store";
export { buildUISlice } from "./ui-store";
export * from "./types";

export const useAgentStore = create<AgentState>((set, get) => ({
  ...buildSessionSlice({ set, get }),
  ...buildAgentSlice({ set, get }),
  ...buildUISlice(set),
}));

useAgentStore.subscribe((state, prev) => {
  if (state.agentChats !== prev.agentChats) {
    saveUIState(state);
  }
});
