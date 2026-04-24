import type { AgentState } from "./types";

const STORAGE_KEY = "fastclaw:ui-state";
const UI_STATE_VERSION = 1;

export interface PersistedUIState {
  version: number;
  activeAgentId: string;
  agentActiveChats: Record<string, string>;
  agentOpenChats: Record<string, string[]>;
}

export function loadUIState(): PersistedUIState | null {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as PersistedUIState;
    if (parsed.version !== UI_STATE_VERSION) return null;
    return parsed;
  } catch {
    return null;
  }
}

export function saveUIState(state: AgentState) {
  try {
    const agentActiveChats: Record<string, string> = {};
    const agentOpenChats: Record<string, string[]> = {};
    for (const [agentId, ac] of Object.entries(state.agentChats)) {
      agentActiveChats[agentId] = ac.activeChatId;
      agentOpenChats[agentId] = ac.chatList.filter((c) => c.open).map((c) => c.id);
    }
    const persisted: PersistedUIState = {
      version: UI_STATE_VERSION,
      activeAgentId: state.activeAgentId,
      agentActiveChats,
      agentOpenChats,
    };
    localStorage.setItem(STORAGE_KEY, JSON.stringify(persisted));
  } catch { /* ignore quota errors */ }
}

export const _persisted = loadUIState();
