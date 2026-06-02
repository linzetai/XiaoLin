import * as api from "../api";
import {
  createChat, formatTime, idCounter, initAgentChats,
} from "./chat-helpers";
import { _persisted } from "./persistence";
import type {
  AgentState,
  BackendMessage,
  BackendSession,
  Chat,
  ChatMessage,
  ChatMessageImage,
  ChatMessageToolCall,
  ChatStreamSegment,
  ChatUsage,
  ExecutionMode,
  QueuedMessage,
  StreamItem,
  SubAgentRunUI,
  SubAgentToolCall,
} from "./types";

/** SQLite `datetime('now')` stores UTC without a tz suffix; ensure JS parses as UTC. */
function parseUtcTimestamp(ts: string): Date {
  if (!ts) return new Date();
  if (ts.endsWith("Z") || /[+-]\d{2}:\d{2}$/.test(ts)) return new Date(ts);
  return new Date(ts.replace(" ", "T") + "Z");
}

type SetGet = {
  set: (partial: AgentState | Partial<AgentState> | ((s: AgentState) => AgentState | Partial<AgentState>)) => void;
  get: () => AgentState;
};

export function buildSessionSlice({ set, get }: SetGet) {
  return {
    agentChats: initAgentChats(),

    addMessage: (agentId: string, msg: Omit<ChatMessage, "id" | "chatId">, targetChatId?: string) => {
      set((state) => {
        const ac = state.agentChats[agentId];
        if (!ac) return state;
        const targetId = targetChatId ?? ac.activeChatId;
        const chat = ac.chatList.find((c) => c.id === targetId);
        if (!chat) return state;

        const fullMsg: ChatMessage = { ...msg, id: idCounter.nextId++, chatId: chat.id };

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

    newChat: (agentId: string, workDir?: string) => {
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

    setActiveChat: (agentId: string, chatId: string) => {
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

    closeChat: (agentId: string, chatId: string) => {
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

    reopenChat: (agentId: string, chatId: string) => {
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

    setWorkDir: (agentId: string, chatId: string, workDir: string | null) => {
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

    renameChat: (agentId: string, chatId: string, title: string) => {
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

    reorderChats: (agentId: string, fromIdx: number, toIdx: number) => {
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

    clearUnread: (agentId: string) => {
      set((state) => {
        const ac = state.agentChats[agentId];
        if (!ac || ac.unread === 0) return state;
        return {
          agentChats: { ...state.agentChats, [agentId]: { ...ac, unread: 0 } },
        };
      });
    },

    syncSessionsForAgent: (agentId: string, sessions: BackendSession[]) => {
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
            if (backend.workDir !== undefined && backend.workDir !== c.workDir) updates.workDir = backend.workDir ?? null;
            if (backend.source && backend.source !== c.source) updates.source = backend.source;
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
            source: s.source ?? "client",
            stream: [],
            createdAt: parseUtcTimestamp(s.createdAt),
            messageCount: s.messageCount,
            open: persistedOpen.has(s.id) && s.messageCount > 0,
            subAgentRuns: {},
            executionMode: "agent",
            usage: (s.totalPromptTokens || s.totalCompletionTokens || s.totalElapsedMs) ? {
              promptTokens: s.totalPromptTokens ?? 0,
              completionTokens: s.totalCompletionTokens ?? 0,
              totalTokens: (s.totalPromptTokens ?? 0) + (s.totalCompletionTokens ?? 0),
              elapsedMs: s.totalElapsedMs ?? 0,
            } : undefined,
          }));
        const mergedList = [...updatedExisting, ...newChats];

        let activeChatId = ac.activeChatId;
        // Preserve the current activeChatId if it still exists in the merged list
        // (e.g. user is typing in a new local chat that hasn't been synced yet).
        // Only fall back to persisted/recent session when the current ID is gone.
        const currentStillValid = mergedList.some((c) => c.id === activeChatId);
        if (currentStillValid) {
          // Keep current activeChatId — nothing to do
        } else if (persistedActive && mergedList.some((c) => c.id === persistedActive)) {
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

    loadChatStream: (agentId: string, chatId: string, messages: BackendMessage[]) => {
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
            const usage: ChatUsage | undefined =
              (m.promptTokens || m.completionTokens || m.elapsedMs)
                ? {
                    promptTokens: m.promptTokens ?? 0,
                    completionTokens: m.completionTokens ?? 0,
                    totalTokens: m.totalTokens ?? 0,
                    elapsedMs: m.elapsedMs ?? 0,
                  }
                : undefined;
            return {
              type: "message" as const,
              data: {
                role: m.role as "user" | "assistant" | "system",
                content,
                id: idCounter.nextId++,
                timestamp: parseUtcTimestamp(m.createdAt),
                chatId,
                toolCalls,
                images,
                usage,
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

    updateChatBackendId: (agentId: string, localChatId: string, backendSessionId: string) => {
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

    updateChatUsage: (agentId: string, chatId: string, incoming: ChatUsage) => {
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
                const updatedStream = [...c.stream];
                const hasTokenData = incoming.promptTokens > 0 || incoming.completionTokens > 0 || incoming.elapsedMs > 0;
                if (hasTokenData) {
                  for (let i = updatedStream.length - 1; i >= 0; i--) {
                    const item = updatedStream[i];
                    if (item.type === "message" && item.data.role === "assistant") {
                      updatedStream[i] = {
                        ...item,
                        data: { ...item.data, usage: incoming },
                      };
                      break;
                    }
                  }
                }
                return {
                  ...c,
                  stream: updatedStream,
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

    setChatExecutionMode: (agentId: string, chatId: string, mode: ExecutionMode) => {
      set((state) => {
        const ac = state.agentChats[agentId];
        if (!ac) return state;
        return {
          agentChats: {
            ...state.agentChats,
            [agentId]: {
              ...ac,
              chatList: ac.chatList.map((c) =>
                c.id === chatId ? { ...c, executionMode: mode } : c,
              ),
            },
          },
        };
      });
    },

    setChatPlanFile: (agentId: string, chatId: string, path: string, exists: boolean) => {
      set((state) => {
        const ac = state.agentChats[agentId];
        if (!ac) return state;
        return {
          agentChats: {
            ...state.agentChats,
            [agentId]: {
              ...ac,
              chatList: ac.chatList.map((c) =>
                c.id === chatId ? { ...c, planFilePath: path, planFileExists: exists } : c,
              ),
            },
          },
        };
      });
    },

    setChatLastSegments: (agentId: string, chatId: string, segments: ChatStreamSegment[]) => {
      set((state) => {
        const ac = state.agentChats[agentId];
        if (!ac) return state;
        return {
          agentChats: {
            ...state.agentChats,
            [agentId]: {
              ...ac,
              chatList: ac.chatList.map((c) =>
                c.id === chatId ? { ...c, lastSegments: segments } : c,
              ),
            },
          },
        };
      });
    },

    subAgentStart: (agentId: string, chatId: string, run: SubAgentRunUI) => {
      set((state) => {
        const ac = state.agentChats[agentId];
        if (!ac) return state;
        return {
          agentChats: { ...state.agentChats, [agentId]: { ...ac, chatList: ac.chatList.map((c) =>
            c.id !== chatId ? c : { ...c, subAgentRuns: { ...c.subAgentRuns, [run.runId]: run } },
          ) } },
        };
      });
    },

    subAgentDelta: (agentId: string, chatId: string, runId: string, content: string) => {
      set((state) => {
        const ac = state.agentChats[agentId];
        if (!ac) return state;
        return {
          agentChats: { ...state.agentChats, [agentId]: { ...ac, chatList: ac.chatList.map((c) => {
            if (c.id !== chatId) return c;
            const run = c.subAgentRuns[runId];
            if (!run) return c;
            return { ...c, subAgentRuns: { ...c.subAgentRuns, [runId]: { ...run, content: run.content + content } } };
          }) } },
        };
      });
    },

    subAgentToolStart: (agentId: string, chatId: string, runId: string, toolCall: SubAgentToolCall) => {
      set((state) => {
        const ac = state.agentChats[agentId];
        if (!ac) return state;
        return {
          agentChats: { ...state.agentChats, [agentId]: { ...ac, chatList: ac.chatList.map((c) => {
            if (c.id !== chatId) return c;
            const run = c.subAgentRuns[runId];
            if (!run) return c;
            return { ...c, subAgentRuns: { ...c.subAgentRuns, [runId]: { ...run, toolCalls: [...run.toolCalls, toolCall] } } };
          }) } },
        };
      });
    },

    subAgentToolDone: (agentId: string, chatId: string, runId: string, callId: string, output: string, success: boolean) => {
      set((state) => {
        const ac = state.agentChats[agentId];
        if (!ac) return state;
        return {
          agentChats: { ...state.agentChats, [agentId]: { ...ac, chatList: ac.chatList.map((c) => {
            if (c.id !== chatId) return c;
            const run = c.subAgentRuns[runId];
            if (!run) return c;
            const toolCalls = run.toolCalls.map((tc) =>
              tc.id === callId ? { ...tc, result: output, status: (success ? "success" : "error") as "success" | "error" } : tc,
            );
            return { ...c, subAgentRuns: { ...c.subAgentRuns, [runId]: { ...run, toolCalls } } };
          }) } },
        };
      });
    },

    subAgentComplete: (agentId: string, chatId: string, runId: string, status: string, result?: string, toolCallsMade?: number, iterations?: number, elapsedMs?: number) => {
      set((state) => {
        const ac = state.agentChats[agentId];
        if (!ac) return state;
        return {
          agentChats: { ...state.agentChats, [agentId]: { ...ac, chatList: ac.chatList.map((c) => {
            if (c.id !== chatId) return c;
            const run = c.subAgentRuns[runId];
            if (!run) return c;
            return { ...c, subAgentRuns: { ...c.subAgentRuns, [runId]: {
              ...run,
              status: status as SubAgentRunUI["status"],
              result: result ?? run.result,
              toolCallsMade: toolCallsMade ?? run.toolCallsMade,
              iterations: iterations ?? run.iterations,
              elapsedMs: elapsedMs ?? run.elapsedMs,
            } } };
          }) } },
        };
      });
    },

    appendStreamDelta: (agentId: string, chatId: string, delta: string) => {
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
            id: idCounter.nextId++,
            timestamp: new Date(),
            chatId,
          };
          return { ...c, stream: [...c.stream, { type: "message" as const, data: newMsg }] };
        });
        return { agentChats: { ...state.agentChats, [agentId]: { ...ac, chatList } } };
      });
    },

    enqueueMessage: (agentId: string, _chatId: string, message: Omit<QueuedMessage, "id">) => {
      set((state) => {
        const ac = state.agentChats[agentId];
        if (!ac) return state;
        const queue = ac.messageQueue ?? [];
        if (queue.length >= 10) return state; // Max 10 items
        const newMsg: QueuedMessage = {
          ...message,
          id: `queue-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`,
        };
        return {
          agentChats: {
            ...state.agentChats,
            [agentId]: { ...ac, messageQueue: [...queue, newMsg] },
          },
        };
      });
    },

    dequeueMessage: (agentId: string, _chatId: string): QueuedMessage | undefined => {
      let result: QueuedMessage | undefined;
      set((state) => {
        const ac = state.agentChats[agentId];
        if (!ac) return state;
        const queue = ac.messageQueue ?? [];
        if (queue.length === 0) return state;
        result = queue[0];
        return {
          agentChats: {
            ...state.agentChats,
            [agentId]: { ...ac, messageQueue: queue.slice(1) },
          },
        };
      });
      return result;
    },

    updateQueuedMessage: (agentId: string, _chatId: string, messageId: string, updates: Partial<QueuedMessage>) => {
      set((state) => {
        const ac = state.agentChats[agentId];
        if (!ac) return state;
        const queue = ac.messageQueue ?? [];
        return {
          agentChats: {
            ...state.agentChats,
            [agentId]: {
              ...ac,
              messageQueue: queue.map((m) => (m.id === messageId ? { ...m, ...updates } : m)),
            },
          },
        };
      });
    },

    removeQueuedMessage: (agentId: string, _chatId: string, messageId: string) => {
      set((state) => {
        const ac = state.agentChats[agentId];
        if (!ac) return state;
        const queue = ac.messageQueue ?? [];
        return {
          agentChats: {
            ...state.agentChats,
            [agentId]: { ...ac, messageQueue: queue.filter((m) => m.id !== messageId) },
          },
        };
      });
    },

    clearQueue: (agentId: string, _chatId: string) => {
      set((state) => {
        const ac = state.agentChats[agentId];
        if (!ac) return state;
        return {
          agentChats: {
            ...state.agentChats,
            [agentId]: { ...ac, messageQueue: [] },
          },
        };
      });
    },

    reorderQueue: (agentId: string, _chatId: string, fromIndex: number, toIndex: number) => {
      set((state) => {
        const ac = state.agentChats[agentId];
        if (!ac) return state;
        const queue = ac.messageQueue ?? [];
        if (fromIndex < 0 || fromIndex >= queue.length || toIndex < 0 || toIndex >= queue.length) return state;
        const newQueue = [...queue];
        const [moved] = newQueue.splice(fromIndex, 1);
        newQueue.splice(toIndex, 0, moved);
    return {
          agentChats: {
            ...state.agentChats,
            [agentId]: { ...ac, messageQueue: newQueue },
          },
        };
      });
    },
  };
}
