import { DEFAULT_AGENT_ID } from "./chat-helpers";
import type { AgentState } from "./types";

const STORAGE_KEY = "xiaolin:ui-state";
const UI_STATE_VERSION = 2;

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

/**
 * @deprecated Used by the old useAgentStore subscribe. Will be removed after full migration.
 */
export function saveUIState(state: AgentState) {
  try {
    const ac = state.agentChats[DEFAULT_AGENT_ID];
    const persisted: PersistedUIState = {
      version: UI_STATE_VERSION,
      activeAgentId: DEFAULT_AGENT_ID,
      agentActiveChats: ac ? { [DEFAULT_AGENT_ID]: ac.activeChatId } : {},
      agentOpenChats: ac ? { [DEFAULT_AGENT_ID]: ac.chatList.filter((c) => c.open).map((c) => c.id) } : {},
    };
    localStorage.setItem(STORAGE_KEY, JSON.stringify(persisted));
  } catch { /* ignore quota errors */ }
}

export function saveUIStateFromMeta(activeChatId: string, chats: Record<string, { open: boolean }>) {
  try {
    const openIds = Object.entries(chats)
      .filter(([_, c]) => c.open)
      .map(([id]) => id);
    const persisted: PersistedUIState = {
      version: UI_STATE_VERSION,
      activeAgentId: DEFAULT_AGENT_ID,
      agentActiveChats: { [DEFAULT_AGENT_ID]: activeChatId },
      agentOpenChats: { [DEFAULT_AGENT_ID]: openIds },
    };
    localStorage.setItem(STORAGE_KEY, JSON.stringify(persisted));
  } catch { /* ignore quota errors */ }
}

export const _persisted = loadUIState();
