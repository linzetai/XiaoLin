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
  ChatMessageToolCall,
  ChatMessageImage,
} from "./types";

function parseUtcTimestamp(ts: string): Date {
  if (!ts) return new Date();
  if (ts.endsWith("Z") || /[+-]\d{2}:\d{2}$/.test(ts)) return new Date(ts);
  return new Date(ts.replace(" ", "T") + "Z");
}

export const EMPTY_STREAM: StreamItem[] = [];
const EMPTY_SUB_AGENT_RUNS: Record<string, SubAgentRunUI> = {};

export interface StreamState {
  streams: Record<string, StreamItem[]>;
  usage: Record<string, ChatUsage>;
  lastSegments: Record<string, ChatStreamSegment[]>;
  subAgentRuns: Record<string, Record<string, SubAgentRunUI>>;

  addMessage: (chatId: string, msg: Omit<ChatMessage, "id" | "chatId">) => void;
  appendStreamDelta: (chatId: string, delta: string) => void;
  updateChatUsage: (chatId: string, usage: ChatUsage) => void;
  setChatLastSegments: (chatId: string, segments: ChatStreamSegment[]) => void;
  addBriefMessage: (chatId: string, brief: BriefMessageData) => void;
  loadChatStream: (chatId: string, messages: BackendMessage[]) => void;
  initStream: (chatId: string) => void;
  updateStreamKey: (oldId: string, newId: string) => void;
  cleanupStream: (chatId: string) => void;

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
      return { streams, usage, lastSegments, subAgentRuns };
    });
  },

  updateStreamKey: (oldId, newId) => {
    set((state) => {
      if (!state.streams[oldId] && !state.usage[oldId] && !state.lastSegments[oldId] && !state.subAgentRuns[oldId]) {
        return state;
      }
      const streams = { ...state.streams };
      const usage = { ...state.usage };
      const lastSegments = { ...state.lastSegments };
      const subAgentRuns = { ...state.subAgentRuns };

      if (streams[oldId]) { streams[newId] = streams[oldId]; delete streams[oldId]; }
      if (usage[oldId]) { usage[newId] = usage[oldId]; delete usage[oldId]; }
      if (lastSegments[oldId]) { lastSegments[newId] = lastSegments[oldId]; delete lastSegments[oldId]; }
      if (subAgentRuns[oldId]) { subAgentRuns[newId] = subAgentRuns[oldId]; delete subAgentRuns[oldId]; }

      return { streams, usage, lastSegments, subAgentRuns };
    });
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

  loadChatStream: (chatId, messages) => {
    // Collect tool-role messages for backward compat (old sessions without enriched tool_calls)
    const toolResultMap = new Map<string, { content: string; success: boolean }>();
    for (const m of messages) {
      if (m.role === "tool" && m.toolCallId) {
        const content = typeof m.content === "string" ? m.content : JSON.stringify(m.content ?? "");
        toolResultMap.set(m.toolCallId, { content, success: true });
      }
    }

    const items: StreamItem[] = messages
      .filter((m) => m.role === "user" || m.role === "assistant" || m.role === "system")
      .map((m) => {
        let toolCalls: ChatMessageToolCall[] | undefined;
        if (m.toolCallsJson && Array.isArray(m.toolCallsJson) && m.toolCallsJson.length > 0) {
          toolCalls = m.toolCallsJson.map((tc) => {
            const callId = tc.id;
            const hasEnriched = tc.output !== undefined || tc.display_output !== undefined;
            const matched = toolResultMap.get(callId);
            const result = hasEnriched
              ? (tc.display_output ?? tc.output)
              : (matched?.content ?? (tc.output as string | undefined));
            const success = hasEnriched
              ? (tc.success !== false)
              : (matched ? matched.success : true);
            return {
              id: callId,
              name: tc.function?.name ?? "unknown",
              status: (success ? "success" : "error") as "success" | "error",
              args: tc.function?.arguments,
              result,
              displayOutput: tc.display_output,
              duration: tc.duration_ms,
              metadata: tc.metadata,
            };
          });
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
            content, id: idCounter.nextId++, timestamp: parseUtcTimestamp(m.createdAt),
            chatId, toolCalls, images, usage,
          },
        };
      });
    set((state) => ({
      streams: { ...state.streams, [chatId]: items },
    }));
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
}));
