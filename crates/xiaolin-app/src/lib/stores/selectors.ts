import { useAgentStore } from "./index";
import { DEFAULT_AGENT_ID } from "./chat-helpers";

/**
 * Subscribe to the main agent's chat data (single-agent mode).
 */
export function useActiveAgentChats() {
  return useAgentStore((s) => s.agentChats[DEFAULT_AGENT_ID]);
}

/**
 * Subscribe to the active chat's stream.
 */
export function useActiveChatStream() {
  return useAgentStore((s) => {
    const ac = s.agentChats[DEFAULT_AGENT_ID];
    if (!ac) return undefined;
    return ac.chatList.find((c) => c.id === ac.activeChatId);
  });
}

/**
 * Get the chatList for the main agent.
 */
export function useAgentChatList(agentId?: string) {
  const id = agentId ?? DEFAULT_AGENT_ID;
  return useAgentStore((s) => s.agentChats[id]?.chatList);
}
