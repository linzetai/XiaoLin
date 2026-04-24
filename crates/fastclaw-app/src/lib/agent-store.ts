import { create } from "zustand";
import * as api from "./api";

export interface Agent {
  id: string;
  name: string;
  initial: string;
  color: string;
  tagline: string;
  online: boolean;
  model: string;
  avatar?: string;
}

export interface ChatMessageToolCall {
  id: string;
  name: string;
  status: "running" | "success" | "error";
  args?: string;
  result?: string;
  duration?: number;
}

export interface ChatMessageImage {
  url: string;
  alt?: string;
}

export interface ChatMessage {
  role: "user" | "assistant" | "system";
  content: string;
  id: number;
  timestamp: Date;
  chatId: string;
  toolCalls?: ChatMessageToolCall[];
  images?: ChatMessageImage[];
}

export type StreamItem = { type: "message"; data: ChatMessage };

export interface ChatUsage {
  promptTokens: number;
  completionTokens: number;
  totalTokens: number;
  elapsedMs: number;
  contextTokens?: number;
  contextWindow?: number;
}

export interface Chat {
  id: string;
  localKey: string;
  title: string;
  workDir: string | null;
  stream: StreamItem[];
  createdAt: Date;
  messageCount: number;
  open: boolean;
  usage?: ChatUsage;
}

export interface AgentChats {
  chatList: Chat[];
  activeChatId: string;
  unread: number;
  lastMsg: string | null;
  lastTime: string | null;
}

interface AgentState {
  agents: Agent[];
  activeAgentId: string;
  agentChats: Record<string, AgentChats>;
  detailOpen: boolean;

  setActiveAgent: (id: string) => void;
  toggleDetail: () => void;
  closeDetail: () => void;
  addMessage: (agentId: string, msg: Omit<ChatMessage, "id" | "chatId">, targetChatId?: string) => void;
  newChat: (agentId: string, workDir?: string) => void;
  setActiveChat: (agentId: string, chatId: string) => void;
  closeChat: (agentId: string, chatId: string) => void;
  reopenChat: (agentId: string, chatId: string) => void;
  setWorkDir: (agentId: string, chatId: string, workDir: string | null) => void;
  renameChat: (agentId: string, chatId: string, title: string) => void;
  reorderChats: (agentId: string, fromIdx: number, toIdx: number) => void;
  clearUnread: (agentId: string) => void;
  syncAgentsFromBackend: (backendAgents: Array<{ agentId: string; name: string; model: string }>) => void;
  syncSessionsForAgent: (agentId: string, sessions: BackendSession[]) => void;
  loadChatStream: (agentId: string, chatId: string, messages: BackendMessage[]) => void;
  updateChatBackendId: (agentId: string, localChatId: string, backendSessionId: string) => void;
  appendStreamDelta: (agentId: string, chatId: string, delta: string) => void;
  updateChatUsage: (agentId: string, chatId: string, usage: ChatUsage) => void;
  removeAgent: (agentId: string) => void;
}

export interface BackendSession {
  id: string;
  agentId: string;
  title: string | null;
  workDir?: string | null;
  messageCount: number;
  createdAt: string;
  updatedAt: string;
  totalPromptTokens?: number;
  totalCompletionTokens?: number;
  totalElapsedMs?: number;
}

export interface BackendMessage {
  id: number;
  role: string;
  content: unknown;
  name: string | null;
  toolCallId: string | null;
  toolCallsJson?: Array<{ id: string; type: string; function: { name: string; arguments: string } }> | null;
  createdAt: string;
}

let nextId = 1;
const STORAGE_KEY = "fastclaw:ui-state";
const DEFAULT_AGENT_ID = "main";

interface PersistedUIState {
  activeAgentId: string;
  agentActiveChats: Record<string, string>;
  agentOpenChats: Record<string, string[]>;
}

function loadUIState(): PersistedUIState | null {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return null;
    return JSON.parse(raw) as PersistedUIState;
  } catch {
    return null;
  }
}

function saveUIState(state: AgentState) {
  try {
    const agentActiveChats: Record<string, string> = {};
    const agentOpenChats: Record<string, string[]> = {};
    for (const [agentId, ac] of Object.entries(state.agentChats)) {
      agentActiveChats[agentId] = ac.activeChatId;
      agentOpenChats[agentId] = ac.chatList.filter((c) => c.open).map((c) => c.id);
    }
    const persisted: PersistedUIState = {
      activeAgentId: state.activeAgentId,
      agentActiveChats,
      agentOpenChats,
    };
    localStorage.setItem(STORAGE_KEY, JSON.stringify(persisted));
  } catch { /* ignore quota errors */ }
}

const INITIAL_AGENTS: Agent[] = [
  {
    id: DEFAULT_AGENT_ID, name: "Main Agent", initial: "M", color: "var(--tint)",
    tagline: "通用智能助手", online: true, model: "qwen3.5-plus",
  },
];

function createChat(workDir?: string): Chat {
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
  };
}

function initAgentChats(): Record<string, AgentChats> {
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

function formatTime(d: Date): string {
  const now = new Date();
  const diff = now.getTime() - d.getTime();
  if (diff < 60000) return "刚刚";
  if (diff < 3600000) return `${Math.floor(diff / 60000)}分钟前`;
  if (diff < 86400000) return `${Math.floor(diff / 3600000)}小时前`;
  return d.toLocaleDateString("zh-CN", { month: "numeric", day: "numeric" });
}

const _persisted = loadUIState();
const initialActiveAgentId =
  _persisted?.activeAgentId === "default"
    ? DEFAULT_AGENT_ID
    : (_persisted?.activeAgentId ?? DEFAULT_AGENT_ID);

export const useAgentStore = create<AgentState>((set, get) => ({
  agents: INITIAL_AGENTS,
  activeAgentId: initialActiveAgentId,
  agentChats: initAgentChats(),
  detailOpen: false,

  setActiveAgent: (id) => {
    set({ activeAgentId: id });
    get().clearUnread(id);
  },

  toggleDetail: () => set((s) => ({ detailOpen: !s.detailOpen })),
  closeDetail: () => set({ detailOpen: false }),

  addMessage: (agentId, msg, targetChatId) => {
    set((state) => {
      const ac = state.agentChats[agentId];
      if (!ac) return state;
      const targetId = targetChatId ?? ac.activeChatId;
      const chat = ac.chatList.find((c) => c.id === targetId);
      if (!chat) return state;

      const fullMsg: ChatMessage = { ...msg, id: nextId++, chatId: chat.id };

      const updatedChatList = ac.chatList.map((c) =>
        c.id === chat.id
          ? {
              ...c,
              stream: [...c.stream, { type: "message" as const, data: fullMsg }],
              messageCount: c.messageCount + 1,
              title: c.messageCount === 0 && msg.role === "user" ? msg.content.slice(0, 20) : c.title,
            }
          : c,
      );

      const isBackground = state.activeAgentId !== agentId;
      return {
        agentChats: {
          ...state.agentChats,
          [agentId]: {
            ...ac,
            chatList: updatedChatList,
            unread: isBackground && msg.role === "assistant" ? ac.unread + 1 : ac.unread,
            lastMsg: msg.content.length > 30 ? msg.content.slice(0, 30) + "..." : msg.content,
            lastTime: formatTime(new Date()),
          },
        },
      };
    });
  },

  newChat: (agentId, workDir) => {
    set((state) => {
      const ac = state.agentChats[agentId];
      if (!ac) return state;
      const chat = createChat(workDir);
      return {
        agentChats: {
          ...state.agentChats,
          [agentId]: { ...ac, chatList: [...ac.chatList, chat], activeChatId: chat.id },
        },
      };
    });
  },

  setActiveChat: (agentId, chatId) => {
    set((state) => {
      const ac = state.agentChats[agentId];
      if (!ac) return state;
      const chat = ac.chatList.find((c) => c.id === chatId);
      if (!chat) return state;
      return {
        agentChats: {
          ...state.agentChats,
          [agentId]: {
            ...ac,
            activeChatId: chatId,
            chatList: chat.open ? ac.chatList : ac.chatList.map((c) => (c.id === chatId ? { ...c, open: true } : c)),
          },
        },
      };
    });
  },

  closeChat: (agentId, chatId) => {
    const ac = get().agentChats[agentId];
    const chat = ac?.chatList.find((c) => c.id === chatId);
    const isEmpty = chat && chat.messageCount === 0 && chat.stream.length === 0;
    set((state) => {
      const acState = state.agentChats[agentId];
      if (!acState) return state;

      const prevOpen = acState.chatList.filter((c) => c.open);
      const closedIdx = prevOpen.findIndex((c) => c.id === chatId);

      const updated = isEmpty
        ? acState.chatList.filter((c) => c.id !== chatId)
        : acState.chatList.map((c) => (c.id === chatId ? { ...c, open: false } : c));
      const openChats = updated.filter((c) => c.open);
      let newActiveId = acState.activeChatId;
      if (chatId === acState.activeChatId && openChats.length > 0) {
        const nextIdx = Math.min(closedIdx, openChats.length - 1);
        newActiveId = openChats[Math.max(0, nextIdx)]?.id ?? openChats[0].id;
      }
      if (openChats.length === 0) {
        const fresh = createChat();
        updated.push(fresh);
        newActiveId = fresh.id;
      }
      return {
        agentChats: {
          ...state.agentChats,
          [agentId]: { ...acState, chatList: updated, activeChatId: newActiveId },
        },
      };
    });
    if (isEmpty && !chatId.startsWith("chat-")) {
      api.deleteSession(chatId).catch(() => {});
    }
  },

  reopenChat: (agentId, chatId) => {
    set((state) => {
      const ac = state.agentChats[agentId];
      if (!ac) return state;
      return {
        agentChats: {
          ...state.agentChats,
          [agentId]: {
            ...ac,
            chatList: ac.chatList.map((c) => (c.id === chatId ? { ...c, open: true } : c)),
            activeChatId: chatId,
          },
        },
      };
    });
  },

  setWorkDir: (agentId, chatId, workDir) => {
    set((state) => {
      const ac = state.agentChats[agentId];
      if (!ac) return state;
      return {
        agentChats: {
          ...state.agentChats,
          [agentId]: { ...ac, chatList: ac.chatList.map((c) => (c.id === chatId ? { ...c, workDir } : c)) },
        },
      };
    });
    api.setSessionWorkDir(chatId, workDir).catch(() => {});
  },

  renameChat: (agentId, chatId, title) => {
    set((state) => {
      const ac = state.agentChats[agentId];
      if (!ac) return state;
      return {
        agentChats: {
          ...state.agentChats,
          [agentId]: { ...ac, chatList: ac.chatList.map((c) => (c.id === chatId ? { ...c, title } : c)) },
        },
      };
    });
    if (!chatId.startsWith("chat-")) {
      api.updateSessionTitle(chatId, title).catch(() => {});
    }
  },

  reorderChats: (agentId, fromIdx, toIdx) => {
    set((state) => {
      const ac = state.agentChats[agentId];
      if (!ac) return state;
      const openIds = ac.chatList.filter((c) => c.open).map((c) => c.id);
      if (fromIdx < 0 || fromIdx >= openIds.length || toIdx < 0 || toIdx >= openIds.length) return state;
      const fromId = openIds[fromIdx];
      const toId = openIds[toIdx];
      const list = [...ac.chatList];
      const realFrom = list.findIndex((c) => c.id === fromId);
      const realTo = list.findIndex((c) => c.id === toId);
      if (realFrom < 0 || realTo < 0) return state;
      const [moved] = list.splice(realFrom, 1);
      list.splice(realTo, 0, moved);
      return {
        agentChats: { ...state.agentChats, [agentId]: { ...ac, chatList: list } },
      };
    });
  },

  clearUnread: (agentId) => {
    set((state) => {
      const ac = state.agentChats[agentId];
      if (!ac || ac.unread === 0) return state;
      return {
        agentChats: { ...state.agentChats, [agentId]: { ...ac, unread: 0 } },
      };
    });
  },

  syncAgentsFromBackend: (backendAgents) => {
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

  syncSessionsForAgent: (agentId, sessions) => {
    set((state) => {
      const ac = state.agentChats[agentId];
      if (!ac) return state;
      const persistedOpen = new Set(_persisted?.agentOpenChats?.[agentId] ?? []);
      const persistedActive = _persisted?.agentActiveChats?.[agentId];

      const existingIds = new Set(ac.chatList.map((c) => c.id));
      const sessionMap = new Map(sessions.map((s) => [s.id, s]));

      const updatedExisting = ac.chatList.map((c) => {
        const backend = sessionMap.get(c.id);
        if (backend) {
          const updates: Partial<Chat> = {};
          if (backend.messageCount > c.messageCount) updates.messageCount = backend.messageCount;
          if (backend.workDir && !c.workDir) updates.workDir = backend.workDir;
          if (Object.keys(updates).length > 0) return { ...c, ...updates };
        }
        return c;
      });

      const newChats: Chat[] = sessions
        .filter((s) => !existingIds.has(s.id))
        .map((s) => ({
          id: s.id,
          localKey: s.id,
          title: s.title || "未命名会话",
          workDir: s.workDir ?? null,
          stream: [],
          createdAt: new Date(s.createdAt),
          messageCount: s.messageCount,
          open: persistedOpen.has(s.id) && s.messageCount > 0,
          usage: (s.totalPromptTokens || s.totalCompletionTokens || s.totalElapsedMs) ? {
            promptTokens: s.totalPromptTokens ?? 0,
            completionTokens: s.totalCompletionTokens ?? 0,
            totalTokens: (s.totalPromptTokens ?? 0) + (s.totalCompletionTokens ?? 0),
            elapsedMs: s.totalElapsedMs ?? 0,
          } : undefined,
        }));
      const mergedList = [...updatedExisting, ...newChats];

      let activeChatId = ac.activeChatId;
      if (persistedActive && mergedList.some((c) => c.id === persistedActive)) {
        activeChatId = persistedActive;
        const target = mergedList.find((c) => c.id === persistedActive);
        if (target) target.open = true;
      } else if (sessions.length > 0) {
        const withMessages = sessions.filter((s) => s.messageCount > 0);
        const mostRecent = withMessages[0] ?? sessions[0];
        activeChatId = mostRecent.id;
        const target = mergedList.find((c) => c.id === mostRecent.id);
        if (target) target.open = true;
      }

      return {
        agentChats: {
          ...state.agentChats,
          [agentId]: { ...ac, chatList: mergedList, activeChatId },
        },
      };
    });
  },

  loadChatStream: (agentId, chatId, messages) => {
    set((state) => {
      const ac = state.agentChats[agentId];
      if (!ac) return state;
      const items: StreamItem[] = messages
        .filter((m) => m.role === "user" || m.role === "assistant" || m.role === "system")
        .map((m) => {
          let toolCalls: ChatMessageToolCall[] | undefined;
          if (m.toolCallsJson && Array.isArray(m.toolCallsJson) && m.toolCallsJson.length > 0) {
            toolCalls = m.toolCallsJson.map((tc: Record<string, unknown>) => ({
              id: tc.id as string,
              name: (tc.function as Record<string, string>)?.name ?? "unknown",
              status: (tc.success === false ? "error" : "success") as "success" | "error",
              args: (tc.function as Record<string, string>)?.arguments,
              result: tc.output as string | undefined,
            }));
          }
          let content: string;
          let images: ChatMessageImage[] | undefined;
          if (Array.isArray(m.content)) {
            const textParts: string[] = [];
            const imgParts: ChatMessageImage[] = [];
            for (const part of m.content as Array<Record<string, unknown>>) {
              if (part.type === "text" && typeof part.text === "string") {
                textParts.push(part.text);
              } else if (part.type === "image_url") {
                const iu = part.image_url as Record<string, string> | undefined;
                if (iu?.url) imgParts.push({ url: iu.url });
              }
            }
            content = textParts.join("\n");
            if (imgParts.length > 0) images = imgParts;
          } else {
            content = typeof m.content === "string" ? m.content : JSON.stringify(m.content ?? "");
          }
          return {
            type: "message" as const,
            data: {
              role: m.role as "user" | "assistant" | "system",
              content,
              id: nextId++,
              timestamp: new Date(m.createdAt),
              chatId,
              toolCalls,
              images,
            },
          };
        });
      return {
        agentChats: {
          ...state.agentChats,
          [agentId]: {
            ...ac,
            chatList: ac.chatList.map((c) =>
              c.id === chatId ? { ...c, stream: items, messageCount: items.length } : c,
            ),
          },
        },
      };
    });
  },

  updateChatBackendId: (agentId, localChatId, backendSessionId) => {
    set((state) => {
      const ac = state.agentChats[agentId];
      if (!ac) return state;
      if (ac.chatList.some((c) => c.id === backendSessionId)) return state;
      return {
        agentChats: {
          ...state.agentChats,
          [agentId]: {
            ...ac,
            chatList: ac.chatList.map((c) => (c.id === localChatId ? { ...c, id: backendSessionId } : c)),
            activeChatId: ac.activeChatId === localChatId ? backendSessionId : ac.activeChatId,
          },
        },
      };
    });
  },

  updateChatUsage: (agentId, chatId, incoming) => {
    set((state) => {
      const ac = state.agentChats[agentId];
      if (!ac) return state;
      return {
        agentChats: {
          ...state.agentChats,
          [agentId]: {
            ...ac,
            chatList: ac.chatList.map((c) => {
              if (c.id !== chatId) return c;
              const prev = c.usage;
              return {
                ...c,
                usage: {
                  promptTokens: (prev?.promptTokens ?? 0) + incoming.promptTokens,
                  completionTokens: (prev?.completionTokens ?? 0) + incoming.completionTokens,
                  totalTokens: (prev?.totalTokens ?? 0) + incoming.totalTokens,
                  elapsedMs: (prev?.elapsedMs ?? 0) + incoming.elapsedMs,
                  contextTokens: incoming.contextTokens ?? prev?.contextTokens,
                  contextWindow: incoming.contextWindow ?? prev?.contextWindow,
                },
              };
            }),
          },
        },
      };
    });
  },

  removeAgent: (agentId) => {
    set((state) => {
      const filtered = state.agents.filter((a) => a.id !== agentId);
      if (filtered.length === 0) return state;
      const { [agentId]: _, ...remainChats } = state.agentChats;
      const newActive = state.activeAgentId === agentId ? filtered[0].id : state.activeAgentId;
      return { agents: filtered, activeAgentId: newActive, agentChats: remainChats };
    });
  },

  appendStreamDelta: (agentId, chatId, delta) => {
    set((state) => {
      const ac = state.agentChats[agentId];
      if (!ac) return state;
      const chatList = ac.chatList.map((c) => {
        if (c.id !== chatId) return c;
        const lastItem = c.stream[c.stream.length - 1];
        if (lastItem && lastItem.data.role === "assistant") {
          const updated = { ...lastItem, data: { ...lastItem.data, content: lastItem.data.content + delta } };
          return { ...c, stream: [...c.stream.slice(0, -1), updated] };
        }
        const newMsg: ChatMessage = {
          role: "assistant",
          content: delta,
          id: nextId++,
          timestamp: new Date(),
          chatId,
        };
        return { ...c, stream: [...c.stream, { type: "message" as const, data: newMsg }] };
      });
      return { agentChats: { ...state.agentChats, [agentId]: { ...ac, chatList } } };
    });
  },
}));


useAgentStore.subscribe((state, prev) => {
  if (
    state.activeAgentId !== prev.activeAgentId ||
    state.agentChats !== prev.agentChats
  ) {
    saveUIState(state);
  }
});
