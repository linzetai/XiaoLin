import type { Agent, AgentChats, Chat } from "./types";

export const idCounter = { nextId: 1 };
export const DEFAULT_AGENT_ID = "main";

export const INITIAL_AGENTS: Agent[] = [
  {
    id: DEFAULT_AGENT_ID, name: "Main Agent", initial: "M", color: "var(--tint)",
    tagline: "通用智能助手", online: true, model: "qwen3.5-plus",
  },
];

export function createChat(workDir?: string): Chat {
  const chatId = `new-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
  return {
    id: chatId,
    localKey: chatId,
    title: "新对话",
    workDir: workDir ?? null,
    stream: [],
    createdAt: new Date(),
    messageCount: 0,
    open: true,
    subAgentRuns: {},
  };
}

export function initAgentChats(): Record<string, AgentChats> {
  const result: Record<string, AgentChats> = {};
  const mainChat = createChat();
  result[DEFAULT_AGENT_ID] = {
    chatList: [mainChat],
    activeChatId: mainChat.id,
    unread: 0,
    lastMsg: null,
    lastTime: null,
  };
  return result;
}

export function formatTime(d: Date): string {
  const now = new Date();
  const diff = now.getTime() - d.getTime();
  if (diff < 60000) return "刚刚";
  if (diff < 3600000) return `${Math.floor(diff / 60000)}分钟前`;
  if (diff < 86400000) return `${Math.floor(diff / 3600000)}小时前`;
  return d.toLocaleDateString("zh-CN", { month: "numeric", day: "numeric" });
}
