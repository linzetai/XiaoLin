import { create } from "zustand";
import { idCounter } from "./chat-helpers";
import type {
  BriefMessageData,
  ChatMessage,
  ChatStreamSegment,
  ChatUsage,
  StreamItem,
  SubAgentRunUI,
  SubAgentNotification,
  SubAgentToolCall,
  BackendMessage,
} from "./types";
import { useTimelineStore } from "./timeline-store";
import * as api from "../api";
import type { TurnDisplayNode } from "../timeline/types";

function unsupportedHistoryNode(chatId: string): TurnDisplayNode {
  const now = Date.now();
  return {
    kind: "system_notice",
    node_id: `unsupported-history-${chatId}`,
    turn_id: "unsupported-history",
    status: "completed",
    created_at_ms: now,
    updated_at_ms: now,
    level: "warning",
    category: "unsupported_history",
    message: "This session was created before canonical timeline replay and cannot be reconstructed.",
  };
}

export const EMPTY_STREAM: StreamItem[] = [];
const EMPTY_SUB_AGENT_RUNS: Record<string, SubAgentRunUI> = {};
const MAX_CACHED_STREAMS = 8;

export interface StreamState {
  streams: Record<string, StreamItem[]>;
  usage: Record<string, ChatUsage>;
  lastSegments: Record<string, ChatStreamSegment[]>;
  subAgentRuns: Record<string, Record<string, SubAgentRunUI>>;
  toolProgress: Record<string, { progress?: number; message?: string }>;
  hasMore: Record<string, boolean>;

  addMessage: (chatId: string, msg: Omit<ChatMessage, "id" | "chatId">) => void;
  appendStreamDelta: (chatId: string, delta: string) => void;
  updateChatUsage: (chatId: string, usage: ChatUsage) => void;
  setChatLastSegments: (chatId: string, segments: ChatStreamSegment[]) => void;
  addBriefMessage: (chatId: string, brief: BriefMessageData) => void;
  loadChatStream: (chatId: string, messages: BackendMessage[], hasMore?: boolean) => void;
  prependChatStream: (chatId: string, messages: BackendMessage[], hasMore?: boolean) => void;
  initStream: (chatId: string) => void;
  updateStreamKey: (oldId: string, newId: string) => void;
  cleanupStream: (chatId: string) => void;
  setHasMore: (chatId: string, hasMore: boolean) => void;
  setToolProgress: (callId: string, data: { progress?: number; message?: string }) => void;
  clearToolProgress: () => void;

  subAgentStart: (chatId: string, run: SubAgentRunUI) => void;
  subAgentDelta: (chatId: string, runId: string, content: string) => void;
  subAgentToolStart: (chatId: string, runId: string, toolCall: SubAgentToolCall) => void;
  subAgentToolDone: (chatId: string, runId: string, callId: string, output: string, success: boolean) => void;
  subAgentComplete: (chatId: string, runId: string, status: string, result?: string, toolCallsMade?: number, iterations?: number, elapsedMs?: number) => void;
  subAgentNotification: (chatId: string, runId: string, message: string) => void;
}

export const useStreamStore = create<StreamState>((set) => ({
  streams: {},
  usage: {},
  lastSegments: {},
  subAgentRuns: {},
  toolProgress: {},
  hasMore: {},

  initStream: (chatId) => {
    set((state) => {
      if (state.streams[chatId]) return state;
      return { streams: { ...state.streams, [chatId]: [] } };
    });
  },

  cleanupStream: (chatId) => {
    set((state) => {
      const { [chatId]: _s, ...streams } = state.streams;
      const { [chatId]: _u, ...usage } = state.usage;
      const { [chatId]: _l, ...lastSegments } = state.lastSegments;
      const { [chatId]: _r, ...subAgentRuns } = state.subAgentRuns;
      const { [chatId]: _h, ...hasMore } = state.hasMore;
      return { streams, usage, lastSegments, subAgentRuns, hasMore };
    });
  },

  updateStreamKey: (oldId, newId) => {
    set((state) => {
      if (!state.streams[oldId] && !state.usage[oldId] && !state.lastSegments[oldId] && !state.subAgentRuns[oldId] && state.hasMore[oldId] === undefined) {
        return state;
      }
      const streams = { ...state.streams };
      const usage = { ...state.usage };
      const lastSegments = { ...state.lastSegments };
      const subAgentRuns = { ...state.subAgentRuns };
      const hasMore = { ...state.hasMore };

      if (streams[oldId]) { streams[newId] = streams[oldId]; delete streams[oldId]; }
      if (usage[oldId]) { usage[newId] = usage[oldId]; delete usage[oldId]; }
      if (lastSegments[oldId]) { lastSegments[newId] = lastSegments[oldId]; delete lastSegments[oldId]; }
      if (subAgentRuns[oldId]) { subAgentRuns[newId] = subAgentRuns[oldId]; delete subAgentRuns[oldId]; }
      if (state.hasMore[oldId] !== undefined) { hasMore[newId] = state.hasMore[oldId]; delete hasMore[oldId]; }

      return { streams, usage, lastSegments, subAgentRuns, hasMore };
    });
  },

  setHasMore: (chatId, hasMore) => {
    set((state) => ({ hasMore: { ...state.hasMore, [chatId]: hasMore } }));
  },

  addMessage: (chatId, msg) => {
    set((state) => {
      const stream = state.streams[chatId] ?? [];
      const fullMsg: ChatMessage = { ...msg, id: idCounter.nextId++, chatId };
      return {
        streams: {
          ...state.streams,
          [chatId]: [...stream, { type: "message" as const, data: fullMsg }],
        },
      };
    });
  },

  appendStreamDelta: (chatId, delta) => {
    set((state) => {
      const stream = state.streams[chatId];
      if (!stream) return state;
      const lastItem = stream[stream.length - 1];
      if (lastItem && lastItem.type === "message" && lastItem.data.role === "assistant") {
        const updated = { ...lastItem, data: { ...lastItem.data, content: lastItem.data.content + delta } };
        return { streams: { ...state.streams, [chatId]: [...stream.slice(0, -1), updated] } };
      }
      const newMsg: ChatMessage = {
        role: "assistant", content: delta, id: idCounter.nextId++, timestamp: new Date(), chatId,
      };
      return { streams: { ...state.streams, [chatId]: [...stream, { type: "message" as const, data: newMsg }] } };
    });
  },

  updateChatUsage: (chatId, incoming) => {
    set((state) => {
      const prev = state.usage[chatId];
      const stream = state.streams[chatId];
      let updatedStreams = state.streams;

      if (stream) {
        const hasTokenData = incoming.promptTokens > 0 || incoming.completionTokens > 0 || incoming.elapsedMs > 0;
        if (hasTokenData) {
          const updatedStream = [...stream];
          for (let i = updatedStream.length - 1; i >= 0; i--) {
            const item = updatedStream[i];
            if (item.type === "message" && item.data.role === "assistant") {
              updatedStream[i] = { ...item, data: { ...item.data, usage: incoming } };
              break;
            }
          }
          updatedStreams = { ...state.streams, [chatId]: updatedStream };
        }
      }

      return {
        streams: updatedStreams,
        usage: {
          ...state.usage,
          [chatId]: {
            promptTokens: (prev?.promptTokens ?? 0) + incoming.promptTokens,
            completionTokens: (prev?.completionTokens ?? 0) + incoming.completionTokens,
            totalTokens: (prev?.totalTokens ?? 0) + incoming.totalTokens,
            elapsedMs: (prev?.elapsedMs ?? 0) + incoming.elapsedMs,
            contextTokens: incoming.contextTokens ?? prev?.contextTokens,
            contextWindow: incoming.contextWindow ?? prev?.contextWindow,
          },
        },
      };
    });
  },

  setChatLastSegments: (chatId, segments) => {
    set((state) => ({
      lastSegments: { ...state.lastSegments, [chatId]: segments },
    }));
  },

  addBriefMessage: (chatId, brief) => {
    set((state) => {
      const stream = state.streams[chatId];
      if (!stream) return state;
      return { streams: { ...state.streams, [chatId]: [...stream, { type: "brief" as const, data: brief }] } };
    });
  },

  loadChatStream: (chatId, messages, hasMore) => {
    console.log("[timeline:loadChatStream]", { chatId, msgCount: messages.length, hasMore });
    if (messages.length > 0) {
      useTimelineStore.getState().loadNodes(chatId, [unsupportedHistoryNode(chatId)]);
    }

    // Hydrate the canonical timeline store from display nodes.
    // Legacy message fields are never reconstructed into transcript nodes here.
    api.getSessionDisplayNodes(chatId).then((page) => {
      console.log("[timeline:displayNodes]", { chatId, nodeCount: page.nodes?.length ?? 0 });
      if (page.nodes && page.nodes.length > 0) {
        // Convert raw nodes to TurnDisplayNode[] and load into timeline store.
        const nodes = page.nodes.map((raw) => ({
          ...raw,
          kind: (raw.kind as string) || "system_notice",
        })) as import("../timeline/types").TurnDisplayNode[];
        useTimelineStore.getState().loadNodes(chatId, nodes);
        console.log("[timeline:displayNodes] loaded", { chatId, nodeCount: nodes.length });
      } else {
        console.log("[timeline:displayNodes] empty, falling back to timeline events", { chatId });
        // No display nodes yet — load raw timeline events as fallback.
        api.getSessionTimeline(chatId, undefined, 2000).then((tl) => {
          console.log("[timeline:events]", { chatId, eventCount: tl.events?.length ?? 0 });
          if (tl.events && tl.events.length > 0) {
            useTimelineStore.getState().loadEvents(
              chatId,
              tl.events as unknown as import("../timeline/types").TurnTimelineEvent[],
            );
            console.log("[timeline:events] loaded", { chatId, eventCount: tl.events.length });
          } else {
            console.warn("[timeline:events] empty, showing unsupported history", { chatId });
            useTimelineStore.getState().loadNodes(chatId, [unsupportedHistoryNode(chatId)]);
          }
        }).catch((err) => {
          console.error("[timeline:events] failed", { chatId, error: err });
          useTimelineStore.getState().loadNodes(chatId, [unsupportedHistoryNode(chatId)]);
        });
      }
    }).catch((err) => {
      console.error("[timeline:displayNodes] failed", { chatId, error: err });
      useTimelineStore.getState().loadNodes(chatId, [unsupportedHistoryNode(chatId)]);
    });

    set((state) => {
      const existing = state.streams[chatId];
      if (existing && existing.length > 0) return state;
      const newStreams = { ...state.streams, [chatId]: [] };
      const newUsage = { ...state.usage };
      const newSubRuns = { ...state.subAgentRuns };
      const newHasMore = { ...state.hasMore, [chatId]: hasMore ?? false };

      const keys = Object.keys(newStreams);
      if (keys.length > MAX_CACHED_STREAMS) {
        const evictCount = keys.length - MAX_CACHED_STREAMS;
        const toEvict = keys.filter((k) => k !== chatId).slice(0, evictCount);
        for (const k of toEvict) {
          delete newStreams[k];
          delete newUsage[k];
          delete newSubRuns[k];
          delete newHasMore[k];
        }
      }
      return {
        streams: newStreams,
        usage: newUsage,
        subAgentRuns: newSubRuns,
        hasMore: newHasMore,
      };
    });
  },

  prependChatStream: (chatId, _messages, hasMore) => {
    set((state) => {
      const existing = state.streams[chatId] ?? [];
      const newHasMore = { ...state.hasMore, [chatId]: hasMore ?? false };
      return {
        streams: { ...state.streams, [chatId]: existing },
        hasMore: newHasMore,
      };
    });
  },

  subAgentStart: (chatId, run) => {
    set((state) => ({
      subAgentRuns: {
        ...state.subAgentRuns,
        [chatId]: { ...(state.subAgentRuns[chatId] ?? EMPTY_SUB_AGENT_RUNS), [run.runId]: run },
      },
    }));
  },

  subAgentDelta: (chatId, runId, content) => {
    set((state) => {
      const runs = state.subAgentRuns[chatId];
      const run = runs?.[runId];
      if (!run) return state;
      return {
        subAgentRuns: {
          ...state.subAgentRuns,
          [chatId]: { ...runs, [runId]: { ...run, content: run.content + content } },
        },
      };
    });
  },

  subAgentToolStart: (chatId, runId, toolCall) => {
    set((state) => {
      const runs = state.subAgentRuns[chatId];
      const run = runs?.[runId];
      if (!run) return state;
      return {
        subAgentRuns: {
          ...state.subAgentRuns,
          [chatId]: { ...runs, [runId]: { ...run, toolCalls: [...run.toolCalls, toolCall] } },
        },
      };
    });
  },

  subAgentToolDone: (chatId, runId, callId, output, success) => {
    set((state) => {
      const runs = state.subAgentRuns[chatId];
      const run = runs?.[runId];
      if (!run) return state;
      const toolCalls = run.toolCalls.map((tc) =>
        tc.id === callId ? { ...tc, result: output, status: (success ? "success" : "error") as "success" | "error" } : tc,
      );
      return {
        subAgentRuns: {
          ...state.subAgentRuns,
          [chatId]: { ...runs, [runId]: { ...run, toolCalls } },
        },
      };
    });
  },

  subAgentComplete: (chatId, runId, status, result, toolCallsMade, iterations, elapsedMs) => {
    set((state) => {
      const runs = state.subAgentRuns[chatId];
      const run = runs?.[runId];
      if (!run) return state;
      const streamedCount = run.toolCalls.length;
      const backendCount = toolCallsMade ?? run.toolCallsMade;
      const reconciledCount = Math.max(streamedCount, backendCount);
      return {
        subAgentRuns: {
          ...state.subAgentRuns,
          [chatId]: {
            ...runs,
            [runId]: {
              ...run,
              status: status as SubAgentRunUI["status"],
              result: result ?? run.result,
              toolCallsMade: reconciledCount,
              iterations: iterations ?? run.iterations,
              elapsedMs: elapsedMs ?? run.elapsedMs,
            },
          },
        },
      };
    });
  },

  subAgentNotification: (chatId, runId, message) => {
    set((state) => {
      const runs = state.subAgentRuns[chatId];
      const run = runs?.[runId];
      if (!run) return state;
      const notification: SubAgentNotification = { message, timestamp: Date.now() };
      return {
        subAgentRuns: {
          ...state.subAgentRuns,
          [chatId]: {
            ...runs,
            [runId]: {
              ...run,
              notifications: [...run.notifications, notification],
            },
          },
        },
      };
    });
  },

  setToolProgress: (callId, data) => {
    set((state) => ({
      toolProgress: { ...state.toolProgress, [callId]: data },
    }));
  },

  clearToolProgress: () => {
    set({ toolProgress: {} });
  },
}));
