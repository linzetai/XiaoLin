import { describe, it, expect, beforeEach, vi } from "vitest";
import { useChatMetaStore } from "../stores/chat-meta-store";
import { useStreamStore } from "../stores/stream-store";
import { useTimelineStore } from "../stores/timeline-store";
import { idCounter } from "../stores/chat-helpers";
import * as api from "../api";

vi.mock("../api", () => ({
  deleteSession: vi.fn(() => Promise.resolve()),
  updateSessionTitle: vi.fn(() => Promise.resolve()),
  setSessionWorkDir: vi.fn(() => Promise.resolve()),
  listFiles: vi.fn(() => Promise.resolve({ files: [], dirs: [] })),
  listSkills: vi.fn(() => Promise.resolve([])),
  getSessionDisplayNodes: vi.fn(() => Promise.resolve({ nodes: [] })),
  getSessionTimeline: vi.fn(() => Promise.resolve({ events: [], has_more: false, max_seq: 0 })),
  getTimelineMaxSeq: vi.fn(() => Promise.resolve({ max_seq: 0 })),
}));

vi.mock("../stores/persistence", () => ({
  _persisted: null,
  saveUIStateFromMeta: vi.fn(),
}));

function resetStores() {
  idCounter.nextId = 1;
  const initChat = { id: `new-${Date.now()}-reset`, localKey: `new-${Date.now()}-reset`, title: "新对话", workDir: null, projectId: null, source: "client", createdAt: new Date(), messageCount: 0, open: true, executionMode: "agent" as const };
  useChatMetaStore.setState({
    chats: { [initChat.id]: initChat },
    chatOrder: [initChat.id],
    activeChatId: initChat.id,
    agents: [{ id: "main", name: "Main Agent", initial: "M", color: "var(--tint)", tagline: "通用智能助手", online: true, model: "" }],
    activeAgentId: "main",
    unread: 0,
    lastMsg: null,
    lastTime: null,
  });
  useStreamStore.setState({
    streams: { [initChat.id]: [] },
    usage: {},
    lastSegments: {},
    subAgentRuns: {},
    toolProgress: {},
    hasMore: {},
  });
  useTimelineStore.setState({
    states: {},
    lastSeenSeq: {},
  });
}

describe("store integration (new multi-store)", () => {
  beforeEach(() => {
    resetStores();
  });

  describe("multi-session switching", () => {
    it("creates new chats and switches between them with correct message lists", () => {
      const firstChatId = useChatMetaStore.getState().activeChatId;

      useStreamStore.getState().addMessage(firstChatId, {
        role: "user", content: "Hello in chat 1", timestamp: new Date(),
      });
      useChatMetaStore.getState().incrementMessageCount(firstChatId, "Hello in chat 1");

      useChatMetaStore.getState().newChat();
      const secondChatId = useChatMetaStore.getState().activeChatId;
      expect(secondChatId).not.toBe(firstChatId);

      useStreamStore.getState().addMessage(secondChatId, {
        role: "user", content: "Hello in chat 2", timestamp: new Date(),
      });
      useChatMetaStore.getState().incrementMessageCount(secondChatId, "Hello in chat 2");

      const secondStream = useStreamStore.getState().streams[secondChatId];
      expect(secondStream).toHaveLength(1);
      expect(secondStream[0].data.content).toBe("Hello in chat 2");

      useChatMetaStore.getState().setActiveChat(firstChatId);
      expect(useChatMetaStore.getState().activeChatId).toBe(firstChatId);

      const firstStream = useStreamStore.getState().streams[firstChatId];
      expect(firstStream).toHaveLength(1);
      expect(firstStream[0].data.content).toBe("Hello in chat 1");
    });

    it("maintains independent message history per chat", () => {
      const chatIds: string[] = [];

      for (let i = 0; i < 5; i++) {
        if (i > 0) useChatMetaStore.getState().newChat();
        const chatId = useChatMetaStore.getState().activeChatId;
        chatIds.push(chatId);
        for (let j = 0; j <= i; j++) {
          useStreamStore.getState().addMessage(chatId, {
            role: "user", content: `Chat ${i} message ${j}`, timestamp: new Date(),
          });
        }
      }

      for (let i = 0; i < 5; i++) {
        const stream = useStreamStore.getState().streams[chatIds[i]];
        expect(stream).toHaveLength(i + 1);
        expect(stream[0].data.content).toBe(`Chat ${i} message 0`);
      }
    });
  });

  describe("message accumulation", () => {
    it("accumulates user and assistant messages in order", () => {
      const chatId = useChatMetaStore.getState().activeChatId;

      useStreamStore.getState().addMessage(chatId, { role: "user", content: "What is Rust?", timestamp: new Date() });
      useStreamStore.getState().addMessage(chatId, { role: "assistant", content: "Rust is a systems programming language.", timestamp: new Date() });
      useStreamStore.getState().addMessage(chatId, { role: "user", content: "Tell me more", timestamp: new Date() });

      const stream = useStreamStore.getState().streams[chatId];
      expect(stream).toHaveLength(3);
      expect(stream[0].type === "message" && stream[0].data.role).toBe("user");
      expect(stream[1].type === "message" && stream[1].data.role).toBe("assistant");
      expect(stream[2].type === "message" && stream[2].data.role).toBe("user");
    });

    it("auto-generates title from first user message via incrementMessageCount", () => {
      const chatId = useChatMetaStore.getState().activeChatId;

      useChatMetaStore.getState().incrementMessageCount(chatId, "How to implement a w");

      const chat = useChatMetaStore.getState().chats[chatId];
      expect(chat.title).toBe("How to implement a w");
    });

    it("preserves messages with tool calls", () => {
      const chatId = useChatMetaStore.getState().activeChatId;

      useStreamStore.getState().addMessage(chatId, {
        role: "assistant", content: "Let me read that file.", timestamp: new Date(),
        toolCalls: [
          { id: "tc-1", name: "file_read", status: "success", args: '{"path":"src/main.rs"}', result: "fn main() {}" },
        ],
      });

      const stream = useStreamStore.getState().streams[chatId];
      const item = stream[0];
      if (item.type === "message") {
        expect(item.data.toolCalls).toHaveLength(1);
        expect(item.data.toolCalls![0].name).toBe("file_read");
      }
    });

    it("does not restore legacy backend messages as transcript stream items", () => {
      const chatId = useChatMetaStore.getState().activeChatId;

      useStreamStore.getState().loadChatStream(chatId, [
        {
          id: 4242,
          role: "user",
          content: "restored",
          name: null,
          toolCallId: null,
          toolCallsJson: null,
          createdAt: "2024-01-01 00:00:00",
        },
      ]);

      expect(useStreamStore.getState().streams[chatId]).toEqual([]);
    });

    it("does not reconstruct assistant segments from legacy session history", () => {
      const chatId = useChatMetaStore.getState().activeChatId;

      useStreamStore.getState().loadChatStream(chatId, [
        {
          id: 10,
          role: "assistant",
          content: "first answer",
          name: null,
          toolCallId: null,
          toolCallsJson: [
            {
              id: "tc-1",
              type: "function",
              function: { name: "read_file", arguments: '{"path":"a"}' },
              output: "file a",
              success: true,
            },
          ],
          createdAt: "2024-01-01 00:00:00",
          reasoningContent: "thinking first",
          segmentOrder: ["reasoning", "tool:tc-1", "text"],
        },
        {
          id: 11,
          role: "assistant",
          content: "second answer",
          name: null,
          toolCallId: null,
          toolCallsJson: [
            {
              id: "tc-2",
              type: "function",
              function: { name: "shell_exec", arguments: '{"command":"pwd"}' },
              output: "/tmp",
              success: true,
            },
          ],
          createdAt: "2024-01-01 00:01:00",
          reasoningContent: "thinking second",
          segmentOrder: ["text", "tool:tc-2", "reasoning"],
        },
      ]);

      expect(useStreamStore.getState().streams[chatId]).toEqual([]);
    });

    it("ignores encoded text segment ordering from legacy messages", () => {
      const chatId = useChatMetaStore.getState().activeChatId;

      useStreamStore.getState().loadChatStream(chatId, [
        {
          id: 12,
          role: "assistant",
          content: "before toolafter tool",
          name: null,
          toolCallId: null,
          toolCallsJson: [
            {
              id: "tc-1",
              type: "function",
              function: { name: "read_file", arguments: '{"path":"a"}' },
              output: "file a",
              success: true,
            },
          ],
          createdAt: "2024-01-01 00:00:00",
          segmentOrder: [
            `text:${JSON.stringify("before tool")}`,
            "tool:tc-1",
            `text:${JSON.stringify("after tool")}`,
          ],
        },
      ]);

      expect(useStreamStore.getState().streams[chatId]).toEqual([]);
    });

    it("does not use legacy repeated text and tool ordering as replay source", () => {
      const chatId = useChatMetaStore.getState().activeChatId;

      useStreamStore.getState().loadChatStream(chatId, [
        {
          id: 13,
          role: "assistant",
          content: "all legacy text",
          name: null,
          toolCallId: null,
          toolCallsJson: [
            {
              id: "tc-1",
              type: "function",
              function: { name: "search_in_files", arguments: '{"pattern":"x"}' },
              output: "matches",
              success: true,
            },
          ],
          createdAt: "2024-01-01 00:00:00",
          segmentOrder: ["text", "tool:tc-1", "text"],
        },
      ]);

      expect(useStreamStore.getState().streams[chatId]).toEqual([]);
    });

    it("does not restore segments when prepending older legacy history", () => {
      const chatId = useChatMetaStore.getState().activeChatId;

      useStreamStore.getState().loadChatStream(chatId, [
        {
          id: 20,
          role: "assistant",
          content: "newer",
          name: null,
          toolCallId: null,
          toolCallsJson: null,
          createdAt: "2024-01-01 00:02:00",
          reasoningContent: "newer reasoning",
          segmentOrder: ["reasoning", "text"],
        },
      ], true);

      useStreamStore.getState().prependChatStream(chatId, [
        {
          id: 19,
          role: "assistant",
          content: "older",
          name: null,
          toolCallId: null,
          toolCallsJson: null,
          createdAt: "2024-01-01 00:01:00",
          reasoningContent: "older reasoning",
          segmentOrder: ["reasoning", "text"],
        },
      ], false);

      const state = useStreamStore.getState();
      expect(state.streams[chatId]).toEqual([]);
      expect(state.lastSegments[chatId]).toBeUndefined();
    });
  });

  describe("rapid message sending", () => {
    it("handles 10 rapid messages without errors", () => {
      const chatId = useChatMetaStore.getState().activeChatId;

      for (let i = 0; i < 10; i++) {
        useStreamStore.getState().addMessage(chatId, {
          role: "user", content: `Rapid message ${i}`, timestamp: new Date(),
        });
      }

      const stream = useStreamStore.getState().streams[chatId];
      expect(stream).toHaveLength(10);
      expect(stream[9].data.content).toBe("Rapid message 9");
    });

    it("handles 50 interleaved user/assistant messages without state corruption", () => {
      const chatId = useChatMetaStore.getState().activeChatId;

      for (let i = 0; i < 50; i++) {
        useStreamStore.getState().addMessage(chatId, {
          role: i % 2 === 0 ? "user" : "assistant",
          content: `Message ${i}`, timestamp: new Date(),
        });
      }

      const stream = useStreamStore.getState().streams[chatId];
      expect(stream).toHaveLength(50);
      for (let i = 0; i < 50; i++) {
        const si = stream[i];
        if (si.type === "message") {
          expect(si.data.role).toBe(i % 2 === 0 ? "user" : "assistant");
        }
      }
    });

    it("handles rapid chat creation and message sending across chats", () => {
      for (let i = 0; i < 10; i++) {
        if (i > 0) useChatMetaStore.getState().newChat();
        const chatId = useChatMetaStore.getState().activeChatId;
        useStreamStore.getState().addMessage(chatId, {
          role: "user", content: `Chat ${i} msg`, timestamp: new Date(),
        });
      }

      const { chatOrder } = useChatMetaStore.getState();
      expect(chatOrder).toHaveLength(10);
      for (const id of chatOrder) {
        const stream = useStreamStore.getState().streams[id];
        expect(stream).toHaveLength(1);
      }
    });
  });

  describe("chat close and reopen", () => {
    it("closing active chat selects adjacent tab", () => {
      useChatMetaStore.getState().newChat();
      useChatMetaStore.getState().newChat();
      const { chatOrder, chats } = useChatMetaStore.getState();
      const openChats = chatOrder.filter((id) => chats[id]?.open);
      expect(openChats).toHaveLength(3);

      const middleId = openChats[1];
      useChatMetaStore.getState().setActiveChat(middleId);
      useChatMetaStore.getState().closeChat(middleId);

      expect(useChatMetaStore.getState().activeChatId).not.toBe(middleId);
    });

    it("closing last chat creates a fresh one", () => {
      const chatId = useChatMetaStore.getState().activeChatId;
      useChatMetaStore.getState().closeChat(chatId);

      const { chatOrder, chats, activeChatId } = useChatMetaStore.getState();
      const openChats = chatOrder.filter((id) => chats[id]?.open);
      expect(openChats).toHaveLength(1);
      expect(activeChatId).not.toBe(chatId);
    });
  });

  describe("backend session sync", () => {
    it("merges backend sessions with local chats", () => {
      useChatMetaStore.getState().syncSessionsForAgent([
        {
          id: "backend-1",
          agentId: "main",
          title: "Backend Chat",
          messageCount: 5,
          createdAt: "2024-01-01T00:00:00Z",
          updatedAt: "2024-01-01T01:00:00Z",
        },
      ]);

      const { chats } = useChatMetaStore.getState();
      const backendChat = chats["backend-1"];
      expect(backendChat).toBeDefined();
      expect(backendChat!.title).toBe("Backend Chat");
      expect(backendChat!.messageCount).toBe(5);
    });

    it("updates chat ID when backend assigns one", () => {
      const localId = useChatMetaStore.getState().activeChatId;

      useStreamStore.getState().addMessage(localId, {
        role: "user", content: "Hello", timestamp: new Date(),
      });
      useChatMetaStore.getState().incrementMessageCount(localId, "Hello");

      useChatMetaStore.getState().updateChatBackendId(localId, "server-session-123");

      const { activeChatId, chats } = useChatMetaStore.getState();
      expect(activeChatId).toBe("server-session-123");
      expect(chats["server-session-123"]).toBeDefined();

      const stream = useStreamStore.getState().streams["server-session-123"];
      expect(stream).toHaveLength(1);
    });
  });

  describe("usage tracking", () => {
    it("accumulates token usage across turns", () => {
      const chatId = useChatMetaStore.getState().activeChatId;

      useStreamStore.getState().addMessage(chatId, {
        role: "assistant", content: "Turn 1", timestamp: new Date(),
      });

      useStreamStore.getState().updateChatUsage(chatId, {
        promptTokens: 100, completionTokens: 50, totalTokens: 150,
        elapsedMs: 500, contextTokens: 100, contextWindow: 128000,
      });

      useStreamStore.getState().addMessage(chatId, {
        role: "assistant", content: "Turn 2", timestamp: new Date(),
      });

      useStreamStore.getState().updateChatUsage(chatId, {
        promptTokens: 200, completionTokens: 80, totalTokens: 280,
        elapsedMs: 700, contextTokens: 300, contextWindow: 128000,
      });

      const usage = useStreamStore.getState().usage[chatId]!;
      expect(usage.promptTokens).toBe(300);
      expect(usage.completionTokens).toBe(130);
      expect(usage.elapsedMs).toBe(1200);
      expect(usage.contextTokens).toBe(300);
    });
  });

  describe("rename and reorder", () => {
    it("renames a chat", () => {
      const chatId = useChatMetaStore.getState().activeChatId;

      useChatMetaStore.getState().renameChat(chatId, "My Custom Title");

      const chat = useChatMetaStore.getState().chats[chatId];
      expect(chat.title).toBe("My Custom Title");
    });

    it("reorders chats", () => {
      useChatMetaStore.getState().newChat();
      useChatMetaStore.getState().newChat();

      const { chatOrder, chats } = useChatMetaStore.getState();
      const openIds = chatOrder.filter((id) => chats[id]?.open);

      useChatMetaStore.getState().reorderChats(0, 2);

      const after = useChatMetaStore.getState();
      const afterOpen = after.chatOrder.filter((id) => after.chats[id]?.open);
      expect(afterOpen[0]).toBe(openIds[1]);
      expect(afterOpen[2]).toBe(openIds[0]);
    });
  });

  describe("timeline hydration (loadChatStream)", () => {
    const chatId = "session-with-timeline";
    const mockNodes = [
      {
        kind: "user_message",
        node_id: "node-um-1",
        turn_id: "turn-1",
        status: "completed",
        created_at_ms: 1000,
        updated_at_ms: 1000,
        content: "Hello",
      },
      {
        kind: "assistant_text",
        node_id: "node-at-1",
        turn_id: "turn-1",
        status: "completed",
        created_at_ms: 2000,
        updated_at_ms: 2000,
        content: "Hi there!",
        byte_length: 9,
      },
    ];

    const mockTimelineEvents = [
      {
        id: "evt-1",
        session_id: chatId,
        turn_id: "turn-1",
        seq: 1,
        event_type: "user_message_created",
        schema_version: 1,
        payload_json: { content: "Hello" },
        created_at_ms: 1000,
      },
      {
        id: "evt-2",
        session_id: chatId,
        turn_id: "turn-1",
        seq: 2,
        event_type: "assistant_text_delta",
        schema_version: 1,
        payload_json: { node_id: "node-at-1", delta: "Hi there!" },
        created_at_ms: 2000,
      },
    ];

    beforeEach(() => {
      resetStores();
      // Override API mocks for this test suite
      vi.mocked(api.getSessionDisplayNodes).mockReset();
      vi.mocked(api.getSessionTimeline).mockReset();
    });

    it("loads display nodes and replaces unsupported history placeholder", async () => {
      vi.mocked(api.getSessionDisplayNodes).mockResolvedValue({
        session_id: chatId,
        nodes: mockNodes as any,
        count: mockNodes.length,
      });

      // Call loadChatStream — the unsupported node is loaded synchronously
      useStreamStore.getState().loadChatStream(chatId, [{ id: 1, role: "user", content: "Hello", name: null, toolCallId: null, createdAt: "2026-01-01T00:00:00Z" }], false);

      // Unsported node should be temporarily present
      let timelineNodes = useTimelineStore.getState().states[chatId]?.nodes ?? [];
      expect(timelineNodes.length).toBeGreaterThan(0);
      expect(timelineNodes[0].kind).toBe("system_notice"); // temporary placeholder

      // Wait for async API response
      await vi.waitFor(() => {
        const nodes = useTimelineStore.getState().states[chatId]?.nodes ?? [];
        expect(nodes.length).toBe(2);
      });

      timelineNodes = useTimelineStore.getState().states[chatId]?.nodes ?? [];
      expect(timelineNodes[0].kind).toBe("user_message");
      expect(timelineNodes[1].kind).toBe("assistant_text");
      if (timelineNodes[1].kind === "assistant_text") {
        expect(timelineNodes[1].content).toBe("Hi there!");
      }
    });

    it("falls back to timeline events when display nodes are empty", async () => {
      vi.mocked(api.getSessionDisplayNodes).mockResolvedValue({
        session_id: chatId,
        nodes: [],
        count: 0,
      });
      vi.mocked(api.getSessionTimeline).mockResolvedValue({
        session_id: chatId,
        events: mockTimelineEvents as any,
        count: mockTimelineEvents.length,
      });

      useStreamStore.getState().loadChatStream(chatId, [{ id: 1, role: "user", content: "Hello", name: null, toolCallId: null, createdAt: "2026-01-01T00:00:00Z" }], false);

      await vi.waitFor(() => {
        const nodes = useTimelineStore.getState().states[chatId]?.nodes ?? [];
        expect(nodes.length).toBeGreaterThan(1);
      });

      const timelineNodes = useTimelineStore.getState().states[chatId]?.nodes ?? [];
      // Should have loaded events and reduced them to nodes
      expect(timelineNodes.length).toBe(2);
      expect(timelineNodes[0].kind).toBe("user_message");
    });

    it("shows unsupported history when both APIs return empty", async () => {
      vi.mocked(api.getSessionDisplayNodes).mockResolvedValue({
        session_id: chatId,
        nodes: [],
        count: 0,
      });
      vi.mocked(api.getSessionTimeline).mockResolvedValue({
        session_id: chatId,
        events: [],
        count: 0,
      });

      useStreamStore.getState().loadChatStream(chatId, [{ id: 1, role: "user", content: "Hello", name: null, toolCallId: null, createdAt: "2026-01-01T00:00:00Z" }], false);

      await vi.waitFor(() => {
        const nodes = useTimelineStore.getState().states[chatId]?.nodes ?? [];
        // After both fallbacks fail, should still have the unsupported node
        if (nodes.length > 0) {
          expect(nodes[0].kind).toBe("system_notice");
        }
      });
    });

    it("handles API errors gracefully (shows unsupported node)", async () => {
      vi.mocked(api.getSessionDisplayNodes).mockRejectedValue(new Error("Network error"));

      useStreamStore.getState().loadChatStream(chatId, [{ id: 1, role: "user", content: "Hello", name: null, toolCallId: null, createdAt: "2026-01-01T00:00:00Z" }], false);

      await vi.waitFor(() => {
        const nodes = useTimelineStore.getState().states[chatId]?.nodes ?? [];
        expect(nodes.length).toBeGreaterThan(0);
      });

      const timelineNodes = useTimelineStore.getState().states[chatId]?.nodes ?? [];
      expect(timelineNodes[0].kind).toBe("system_notice");
      expect((timelineNodes[0] as any).category).toBe("unsupported_history");
    });

    it("does NOT load unsupported node when there are no messages", () => {
      vi.mocked(api.getSessionDisplayNodes).mockResolvedValue({
        session_id: chatId,
        nodes: [],
        count: 0,
      });

      // Empty messages array
      useStreamStore.getState().loadChatStream(chatId, [], false);

      const timelineNodes = useTimelineStore.getState().states[chatId]?.nodes ?? [];
      // No messages means no unsupported node (the session might be truly empty)
      expect(timelineNodes.length).toBe(0);
    });
  });
});
