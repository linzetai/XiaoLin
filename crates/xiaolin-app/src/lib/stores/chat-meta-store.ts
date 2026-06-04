import { create } from "zustand";
import * as api from "../api";
import { DEFAULT_AGENT_ID, INITIAL_AGENTS, formatTime } from "./chat-helpers";
import { _persisted } from "./persistence";
import { useStreamStore } from "./stream-store";
import type {
  Agent,
  BackendSession,
  ChatMeta,
  ExecutionMode,
} from "./types";

function parseUtcTimestamp(ts: string): Date {
  if (!ts) return new Date();
  if (ts.endsWith("Z") || /[+-]\d{2}:\d{2}$/.test(ts)) return new Date(ts);
  return new Date(ts.replace(" ", "T") + "Z");
}

function createChatMeta(workDir?: string): ChatMeta {
  const chatId = `new-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
  return {
    id: chatId,
    localKey: chatId,
    title: "新对话",
    workDir: workDir ?? null,
    projectId: null,
    source: "client",
    createdAt: new Date(),
    messageCount: 0,
    open: true,
    executionMode: "agent",
  };
}

export interface ChatMetaState {
  chats: Record<string, ChatMeta>;
  chatOrder: string[];
  activeChatId: string;
  agents: Agent[];
  activeAgentId: string;
  unread: number;
  lastMsg: string | null;
  lastTime: string | null;

  newChat: (workDir?: string) => void;
  setActiveChat: (chatId: string) => void;
  closeChat: (chatId: string) => void;
  reopenChat: (chatId: string) => void;
  renameChat: (chatId: string, title: string) => void;
  reorderChats: (fromIdx: number, toIdx: number) => void;
  setWorkDir: (chatId: string, workDir: string | null) => void;
  clearUnread: () => void;
  incrementMessageCount: (chatId: string, title?: string) => void;
  incrementUnread: (content: string) => void;
  setMessageCount: (chatId: string, count: number) => void;

  syncSessionsForAgent: (sessions: BackendSession[]) => void;
  updateChatBackendId: (localChatId: string, backendSessionId: string) => void;
  setChatExecutionMode: (chatId: string, mode: ExecutionMode) => void;
  setChatPlanFile: (chatId: string, path: string, exists: boolean) => void;

  syncAgentsFromBackend: (backendAgents: Array<{ agentId: string; name: string; model: string; avatar?: string | null }>) => void;
  updateAgentProps: (props: Partial<Pick<Agent, "name" | "model" | "avatar">>) => void;
  setActiveAgent: (id: string) => void;
  removeAgent: (agentId: string) => void;
}

const initialChat = createChatMeta();

export const useChatMetaStore = create<ChatMetaState>((set, get) => ({
  chats: { [initialChat.id]: initialChat },
  chatOrder: [initialChat.id],
  activeChatId: initialChat.id,
  agents: INITIAL_AGENTS as Agent[],
  activeAgentId: DEFAULT_AGENT_ID,
  unread: 0,
  lastMsg: null,
  lastTime: null,

  newChat: (workDir) => {
    const chat = createChatMeta(workDir);
    useStreamStore.getState().initStream(chat.id);
    set((state) => ({
      chats: { ...state.chats, [chat.id]: chat },
      chatOrder: [...state.chatOrder, chat.id],
      activeChatId: chat.id,
    }));
  },

  setActiveChat: (chatId) => {
    set((state) => {
      const chat = state.chats[chatId];
      if (!chat) return state;
      const updates: Partial<ChatMetaState> = { activeChatId: chatId };
      if (!chat.open) {
        updates.chats = { ...state.chats, [chatId]: { ...chat, open: true } };
      }
      return updates;
    });
  },

  closeChat: (chatId) => {
    const state = get();
    const chat = state.chats[chatId];
    if (!chat) return;
    const stream = useStreamStore.getState().streams[chatId];
    const isEmpty = chat.messageCount === 0 && (!stream || stream.length === 0);

    set((s) => {
      const openIds = s.chatOrder.filter((id) => s.chats[id]?.open);
      const closedIdx = openIds.indexOf(chatId);

      let newChats: Record<string, ChatMeta>;
      let newOrder: string[];
      if (isEmpty) {
        const { [chatId]: _, ...rest } = s.chats;
        newChats = rest;
        newOrder = s.chatOrder.filter((id) => id !== chatId);
        useStreamStore.getState().cleanupStream(chatId);
      } else {
        newChats = { ...s.chats, [chatId]: { ...chat, open: false } };
        newOrder = s.chatOrder;
      }

      const openChats = newOrder.filter((id) => newChats[id]?.open);
      let newActiveId = s.activeChatId;

      if (chatId === s.activeChatId && openChats.length > 0) {
        const nextIdx = Math.min(closedIdx, openChats.length - 1);
        newActiveId = openChats[Math.max(0, nextIdx)] ?? openChats[0];
      }
      if (openChats.length === 0) {
        const fresh = createChatMeta();
        useStreamStore.getState().initStream(fresh.id);
        newChats[fresh.id] = fresh;
        newOrder = [...newOrder, fresh.id];
        newActiveId = fresh.id;
      }
      return { chats: newChats, chatOrder: newOrder, activeChatId: newActiveId };
    });

    if (isEmpty && !chatId.startsWith("chat-") && !chatId.startsWith("new-")) {
      api.deleteSession(chatId).catch(() => {});
    }
  },

  reopenChat: (chatId) => {
    set((state) => {
      const chat = state.chats[chatId];
      if (!chat) return state;
      return {
        chats: { ...state.chats, [chatId]: { ...chat, open: true } },
        activeChatId: chatId,
      };
    });
  },

  renameChat: (chatId, title) => {
    set((state) => {
      const chat = state.chats[chatId];
      if (!chat) return state;
      return { chats: { ...state.chats, [chatId]: { ...chat, title } } };
    });
    if (!chatId.startsWith("chat-") && !chatId.startsWith("new-")) {
      api.updateSessionTitle(chatId, title).catch(() => {});
    }
  },

  reorderChats: (fromIdx, toIdx) => {
    set((state) => {
      const openIds = state.chatOrder.filter((id) => state.chats[id]?.open);
      if (fromIdx < 0 || fromIdx >= openIds.length || toIdx < 0 || toIdx >= openIds.length) return state;
      const fromId = openIds[fromIdx];
      const toId = openIds[toIdx];
      const order = [...state.chatOrder];
      const realFrom = order.indexOf(fromId);
      const realTo = order.indexOf(toId);
      if (realFrom < 0 || realTo < 0) return state;
      const [moved] = order.splice(realFrom, 1);
      order.splice(realTo, 0, moved);
      return { chatOrder: order };
    });
  },

  setWorkDir: (chatId, workDir) => {
    const chat = get().chats[chatId];
    set((state) => {
      if (!state.chats[chatId]) return state;
      return { chats: { ...state.chats, [chatId]: { ...state.chats[chatId], workDir } } };
    });
    if (chat && chat.messageCount > 0) {
      api.setSessionWorkDir(chatId, workDir).catch(() => {});
    }
  },

  clearUnread: () => {
    set((state) => (state.unread === 0 ? state : { unread: 0 }));
  },

  incrementMessageCount: (chatId, title) => {
    set((state) => {
      const chat = state.chats[chatId];
      if (!chat) return state;
      const updates: Partial<ChatMeta> = { messageCount: chat.messageCount + 1 };
      if (chat.messageCount === 0 && title) {
        updates.title = title;
      }
      return { chats: { ...state.chats, [chatId]: { ...chat, ...updates } } };
    });
  },

  incrementUnread: (content) => {
    set((state) => ({
      unread: state.unread + 1,
      lastMsg: content.length > 30 ? content.slice(0, 30) + "..." : content,
      lastTime: formatTime(new Date()),
    }));
  },

  setMessageCount: (chatId, count) => {
    set((state) => {
      const chat = state.chats[chatId];
      if (!chat) return state;
      return { chats: { ...state.chats, [chatId]: { ...chat, messageCount: count } } };
    });
  },

  syncSessionsForAgent: (sessions) => {
    set((state) => {
      const persistedOpen = new Set(_persisted?.agentOpenChats?.[DEFAULT_AGENT_ID] ?? []);
      const persistedActive = _persisted?.agentActiveChats?.[DEFAULT_AGENT_ID];
      const existingIds = new Set(state.chatOrder);
      const sessionMap = new Map(sessions.map((s) => [s.id, s]));

      const updatedChats = { ...state.chats };
      for (const id of state.chatOrder) {
        const backend = sessionMap.get(id);
        if (backend) {
          const chat = updatedChats[id];
          const updates: Partial<ChatMeta> = {};
          if (backend.messageCount > chat.messageCount) updates.messageCount = backend.messageCount;
          if (backend.workDir !== undefined && backend.workDir !== chat.workDir) updates.workDir = backend.workDir ?? null;
          if (backend.projectId !== undefined && backend.projectId !== chat.projectId) updates.projectId = backend.projectId ?? null;
          if (backend.source && backend.source !== chat.source) updates.source = backend.source;
          if (Object.keys(updates).length > 0) {
            updatedChats[id] = { ...chat, ...updates };
          }
        }
      }

      const newOrder = [...state.chatOrder];
      for (const s of sessions) {
        if (existingIds.has(s.id)) continue;
        const meta: ChatMeta = {
          id: s.id,
          localKey: s.id,
          title: s.title || "未命名会话",
          workDir: s.workDir ?? null,
          projectId: s.projectId ?? null,
          source: s.source ?? "client",
          createdAt: parseUtcTimestamp(s.createdAt),
          messageCount: s.messageCount,
          open: persistedOpen.has(s.id) && s.messageCount > 0,
          executionMode: "agent",
        };
        updatedChats[s.id] = meta;
        newOrder.push(s.id);
        useStreamStore.getState().initStream(s.id);
        if (s.totalPromptTokens || s.totalCompletionTokens || s.totalElapsedMs) {
          useStreamStore.setState((ss) => ({
            usage: {
              ...ss.usage,
              [s.id]: {
                promptTokens: s.totalPromptTokens ?? 0,
                completionTokens: s.totalCompletionTokens ?? 0,
                totalTokens: (s.totalPromptTokens ?? 0) + (s.totalCompletionTokens ?? 0),
                elapsedMs: s.totalElapsedMs ?? 0,
              },
            },
          }));
        }
      }

      let activeChatId = state.activeChatId;
      const currentStillValid = updatedChats[activeChatId] !== undefined;
      if (!currentStillValid) {
        if (persistedActive && updatedChats[persistedActive]) {
          activeChatId = persistedActive;
          updatedChats[persistedActive] = { ...updatedChats[persistedActive], open: true };
        } else if (sessions.length > 0) {
          const withMessages = sessions.filter((s) => s.messageCount > 0);
          const mostRecent = withMessages[0] ?? sessions[0];
          activeChatId = mostRecent.id;
          if (updatedChats[activeChatId]) {
            updatedChats[activeChatId] = { ...updatedChats[activeChatId], open: true };
          }
        }
      }

      return { chats: updatedChats, chatOrder: newOrder, activeChatId };
    });
  },

  updateChatBackendId: (localChatId, backendSessionId) => {
    set((state) => {
      if (state.chats[backendSessionId]) return state;
      const chat = state.chats[localChatId];
      if (!chat) return state;
      const { [localChatId]: _, ...restChats } = state.chats;
      const updatedChats = { ...restChats, [backendSessionId]: { ...chat, id: backendSessionId } };
      const updatedOrder = state.chatOrder.map((id) => (id === localChatId ? backendSessionId : id));
      const updatedActive = state.activeChatId === localChatId ? backendSessionId : state.activeChatId;
      return { chats: updatedChats, chatOrder: updatedOrder, activeChatId: updatedActive };
    });
    useStreamStore.getState().updateStreamKey(localChatId, backendSessionId);
  },

  setChatExecutionMode: (chatId, mode) => {
    set((state) => {
      const chat = state.chats[chatId];
      if (!chat) return state;
      return { chats: { ...state.chats, [chatId]: { ...chat, executionMode: mode } } };
    });
  },

  setChatPlanFile: (chatId, path, exists) => {
    set((state) => {
      const chat = state.chats[chatId];
      if (!chat) return state;
      return { chats: { ...state.chats, [chatId]: { ...chat, planFilePath: path, planFileExists: exists } } };
    });
  },

  syncAgentsFromBackend: (backendAgents) => {
    const main = backendAgents.find((a) => a.agentId === DEFAULT_AGENT_ID);
    if (!main) return;
    set((state) => ({
      agents: state.agents.map((a) => {
        if (a.id !== DEFAULT_AGENT_ID) return a;
        return { ...a, model: main.model || a.model, name: main.name || a.name };
      }),
    }));
  },

  updateAgentProps: (props) => {
    set((state) => ({
      agents: state.agents.map((a) => {
        if (a.id !== DEFAULT_AGENT_ID) return a;
        const updated = { ...a, ...props };
        if (props.name) updated.initial = props.name.charAt(0).toUpperCase();
        return updated;
      }),
    }));
  },

  setActiveAgent: () => {},
  removeAgent: () => {},
}));
