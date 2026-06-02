import { describe, it, expect, beforeEach, vi } from "vitest";
import { create } from "zustand";
import { buildSessionSlice } from "../stores/session-store";
import { buildAgentSlice } from "../stores/agent-store";
import { buildUISlice } from "../stores/ui-store";
import type { AgentState } from "../stores/types";

vi.mock("../api", () => ({
  deleteSession: vi.fn(() => Promise.resolve()),
  updateSessionTitle: vi.fn(() => Promise.resolve()),
  setSessionWorkDir: vi.fn(() => Promise.resolve()),
  listFiles: vi.fn(() => Promise.resolve({ files: [], dirs: [] })),
  listSkills: vi.fn(() => Promise.resolve([])),
}));

vi.mock("../stores/persistence", () => ({
  _persisted: null,
  saveUIState: vi.fn(),
}));

function createStore() {
  return create<AgentState>((set, get) => ({
    ...buildSessionSlice({ set, get }),
    ...buildAgentSlice({ set, get }),
    ...buildUISlice(set),
  }));
}

describe("session store integration", () => {
  let store: ReturnType<typeof createStore>;

  beforeEach(() => {
    store = createStore();
  });

  // ═══════════════════════════════════════════════════════════════════
  // Multi-session switching (AC3: 切换会话后消息列表正确)
  // ═══════════════════════════════════════════════════════════════════

  describe("multi-session switching", () => {
    it("creates new chats and switches between them with correct message lists", () => {
      const agentId = "main";

      // Initial state: one chat
      const initialAc = store.getState().agentChats[agentId];
      expect(initialAc.chatList).toHaveLength(1);
      const firstChatId = initialAc.activeChatId;

      // Add message to first chat
      store.getState().addMessage(agentId, {
        role: "user",
        content: "Hello in chat 1",
        timestamp: new Date(),
      });

      // Create second chat
      store.getState().newChat(agentId);
      const afterNew = store.getState().agentChats[agentId];
      expect(afterNew.chatList).toHaveLength(2);
      const secondChatId = afterNew.activeChatId;
      expect(secondChatId).not.toBe(firstChatId);

      // Add message to second chat
      store.getState().addMessage(agentId, {
        role: "user",
        content: "Hello in chat 2",
        timestamp: new Date(),
      });

      // Verify second chat has its own message
      const secondChat = store.getState().agentChats[agentId].chatList.find(
        (c) => c.id === secondChatId,
      )!;
      expect(secondChat.stream).toHaveLength(1);
      expect(secondChat.stream[0].data.content).toBe("Hello in chat 2");

      // Switch back to first chat
      store.getState().setActiveChat(agentId, firstChatId);
      expect(store.getState().agentChats[agentId].activeChatId).toBe(firstChatId);

      // Verify first chat still has its original message
      const firstChat = store.getState().agentChats[agentId].chatList.find(
        (c) => c.id === firstChatId,
      )!;
      expect(firstChat.stream).toHaveLength(1);
      expect(firstChat.stream[0].data.content).toBe("Hello in chat 1");
    });

    it("maintains independent message history per chat", () => {
      const agentId = "main";
      const chatIds: string[] = [];

      // Create 5 chats, each with different messages
      for (let i = 0; i < 5; i++) {
        if (i > 0) store.getState().newChat(agentId);
        const chatId = store.getState().agentChats[agentId].activeChatId;
        chatIds.push(chatId);
        for (let j = 0; j <= i; j++) {
          store.getState().addMessage(agentId, {
            role: "user",
            content: `Chat ${i} message ${j}`,
            timestamp: new Date(),
          });
        }
      }

      // Verify each chat has the correct number of messages
      for (let i = 0; i < 5; i++) {
        const chat = store.getState().agentChats[agentId].chatList.find(
          (c) => c.id === chatIds[i],
        )!;
        expect(chat.stream).toHaveLength(i + 1);
        expect(chat.stream[0].data.content).toBe(`Chat ${i} message 0`);
      }
    });
  });

  // ═══════════════════════════════════════════════════════════════════
  // Message accumulation
  // ═══════════════════════════════════════════════════════════════════

  describe("message accumulation", () => {
    it("accumulates user and assistant messages in order", () => {
      const agentId = "main";
      const chatId = store.getState().agentChats[agentId].activeChatId;

      store.getState().addMessage(agentId, {
        role: "user",
        content: "What is Rust?",
        timestamp: new Date(),
      });

      store.getState().addMessage(agentId, {
        role: "assistant",
        content: "Rust is a systems programming language.",
        timestamp: new Date(),
      });

      store.getState().addMessage(agentId, {
        role: "user",
        content: "Tell me more",
        timestamp: new Date(),
      });

      const chat = store.getState().agentChats[agentId].chatList.find(
        (c) => c.id === chatId,
      )!;
      expect(chat.stream).toHaveLength(3);
      expect(chat.stream[0].data.role).toBe("user");
      expect(chat.stream[1].data.role).toBe("assistant");
      expect(chat.stream[2].data.role).toBe("user");
      expect(chat.messageCount).toBe(3);
    });

    it("auto-generates title from first user message", () => {
      const agentId = "main";
      const chatId = store.getState().agentChats[agentId].activeChatId;

      store.getState().addMessage(agentId, {
        role: "user",
        content: "How to implement a web server in Rust?",
        timestamp: new Date(),
      });

      const chat = store.getState().agentChats[agentId].chatList.find(
        (c) => c.id === chatId,
      )!;
      expect(chat.title).toBe("How to implement a w");
    });

    it("preserves messages with tool calls", () => {
      const agentId = "main";
      const chatId = store.getState().agentChats[agentId].activeChatId;

      store.getState().addMessage(agentId, {
        role: "assistant",
        content: "Let me read that file.",
        timestamp: new Date(),
        toolCalls: [
          { id: "tc-1", name: "file_read", status: "success", args: '{"path":"src/main.rs"}', result: "fn main() {}" },
        ],
      });

      const chat = store.getState().agentChats[agentId].chatList.find(
        (c) => c.id === chatId,
      )!;
      expect(chat.stream[0].data.toolCalls).toHaveLength(1);
      expect(chat.stream[0].data.toolCalls![0].name).toBe("file_read");
    });
  });

  // ═══════════════════════════════════════════════════════════════════
  // Rapid message sending (AC4: 连续快速发送 10 条消息不崩溃)
  // ═══════════════════════════════════════════════════════════════════

  describe("rapid message sending", () => {
    it("handles 10 rapid messages without errors", () => {
      const agentId = "main";
      const chatId = store.getState().agentChats[agentId].activeChatId;

      for (let i = 0; i < 10; i++) {
        store.getState().addMessage(agentId, {
          role: "user",
          content: `Rapid message ${i}`,
          timestamp: new Date(),
        });
      }

      const chat = store.getState().agentChats[agentId].chatList.find(
        (c) => c.id === chatId,
      )!;
      expect(chat.stream).toHaveLength(10);
      expect(chat.messageCount).toBe(10);
      expect(chat.stream[9].data.content).toBe("Rapid message 9");
    });

    it("handles 50 interleaved user/assistant messages without state corruption", () => {
      const agentId = "main";
      const chatId = store.getState().agentChats[agentId].activeChatId;

      for (let i = 0; i < 50; i++) {
        store.getState().addMessage(agentId, {
          role: i % 2 === 0 ? "user" : "assistant",
          content: `Message ${i}`,
          timestamp: new Date(),
        });
      }

      const chat = store.getState().agentChats[agentId].chatList.find(
        (c) => c.id === chatId,
      )!;
      expect(chat.stream).toHaveLength(50);
      // Verify alternating roles
      for (let i = 0; i < 50; i++) {
        expect(chat.stream[i].data.role).toBe(i % 2 === 0 ? "user" : "assistant");
      }
    });

    it("handles rapid chat creation and message sending across chats", () => {
      const agentId = "main";

      for (let i = 0; i < 10; i++) {
        if (i > 0) store.getState().newChat(agentId);
        store.getState().addMessage(agentId, {
          role: "user",
          content: `Chat ${i} msg`,
          timestamp: new Date(),
        });
      }

      const ac = store.getState().agentChats[agentId];
      // initial chat gets first msg (i=0), then 9 new chats (i=1..9)
      expect(ac.chatList).toHaveLength(10);
      // Each chat has exactly 1 message
      for (const chat of ac.chatList) {
        expect(chat.stream).toHaveLength(1);
      }
    });
  });

  // ═══════════════════════════════════════════════════════════════════
  // Chat close and reopen
  // ═══════════════════════════════════════════════════════════════════

  describe("chat close and reopen", () => {
    it("closing active chat selects adjacent tab", () => {
      const agentId = "main";
      store.getState().newChat(agentId);
      store.getState().newChat(agentId);
      const ac = store.getState().agentChats[agentId];
      expect(ac.chatList.filter((c) => c.open)).toHaveLength(3);

      const middleId = ac.chatList[1].id;
      store.getState().setActiveChat(agentId, middleId);
      store.getState().closeChat(agentId, middleId);

      const after = store.getState().agentChats[agentId];
      expect(after.activeChatId).not.toBe(middleId);
    });

    it("closing last chat creates a fresh one", () => {
      const agentId = "main";
      const chatId = store.getState().agentChats[agentId].activeChatId;
      store.getState().closeChat(agentId, chatId);

      const after = store.getState().agentChats[agentId];
      expect(after.chatList.filter((c) => c.open)).toHaveLength(1);
      expect(after.activeChatId).not.toBe(chatId);
    });
  });

  // ═══════════════════════════════════════════════════════════════════
  // Backend session sync
  // ═══════════════════════════════════════════════════════════════════

  describe("backend session sync", () => {
    it("merges backend sessions with local chats", () => {
      const agentId = "main";

      store.getState().syncSessionsForAgent(agentId, [
        {
          id: "backend-1",
          agentId,
          title: "Backend Chat",
          messageCount: 5,
          createdAt: "2024-01-01T00:00:00Z",
          updatedAt: "2024-01-01T01:00:00Z",
        },
      ]);

      const ac = store.getState().agentChats[agentId];
      const backendChat = ac.chatList.find((c) => c.id === "backend-1");
      expect(backendChat).toBeDefined();
      expect(backendChat!.title).toBe("Backend Chat");
      expect(backendChat!.messageCount).toBe(5);
    });

    it("updates chat ID when backend assigns one", () => {
      const agentId = "main";
      const localId = store.getState().agentChats[agentId].activeChatId;

      store.getState().addMessage(agentId, {
        role: "user",
        content: "Hello",
        timestamp: new Date(),
      });

      store.getState().updateChatBackendId(agentId, localId, "server-session-123");

      const ac = store.getState().agentChats[agentId];
      expect(ac.activeChatId).toBe("server-session-123");
      const chat = ac.chatList.find((c) => c.id === "server-session-123");
      expect(chat).toBeDefined();
      expect(chat!.stream).toHaveLength(1);
    });
  });

  // ═══════════════════════════════════════════════════════════════════
  // Usage tracking
  // ═══════════════════════════════════════════════════════════════════

  describe("usage tracking", () => {
    it("accumulates token usage across turns", () => {
      const agentId = "main";
      const chatId = store.getState().agentChats[agentId].activeChatId;

      store.getState().addMessage(agentId, {
        role: "assistant",
        content: "Turn 1",
        timestamp: new Date(),
      });

      store.getState().updateChatUsage(agentId, chatId, {
        promptTokens: 100,
        completionTokens: 50,
        totalTokens: 150,
        elapsedMs: 500,
        contextTokens: 100,
        contextWindow: 128000,
      });

      store.getState().addMessage(agentId, {
        role: "assistant",
        content: "Turn 2",
        timestamp: new Date(),
      });

      store.getState().updateChatUsage(agentId, chatId, {
        promptTokens: 200,
        completionTokens: 80,
        totalTokens: 280,
        elapsedMs: 700,
        contextTokens: 300,
        contextWindow: 128000,
      });

      const chat = store.getState().agentChats[agentId].chatList.find(
        (c) => c.id === chatId,
      )!;
      expect(chat.usage!.promptTokens).toBe(300);
      expect(chat.usage!.completionTokens).toBe(130);
      expect(chat.usage!.elapsedMs).toBe(1200);
      expect(chat.usage!.contextTokens).toBe(300);
    });
  });

  // ═══════════════════════════════════════════════════════════════════
  // Rename and reorder
  // ═══════════════════════════════════════════════════════════════════

  describe("rename and reorder", () => {
    it("renames a chat", () => {
      const agentId = "main";
      const chatId = store.getState().agentChats[agentId].activeChatId;

      store.getState().renameChat(agentId, chatId, "My Custom Title");

      const chat = store.getState().agentChats[agentId].chatList.find(
        (c) => c.id === chatId,
      )!;
      expect(chat.title).toBe("My Custom Title");
    });

    it("reorders chats", () => {
      const agentId = "main";
      store.getState().newChat(agentId);
      store.getState().newChat(agentId);

      const before = store.getState().agentChats[agentId].chatList.filter((c) => c.open);
      const ids = before.map((c) => c.id);

      store.getState().reorderChats(agentId, 0, 2);

      const after = store.getState().agentChats[agentId].chatList.filter((c) => c.open);
      expect(after[0].id).toBe(ids[1]);
      expect(after[2].id).toBe(ids[0]);
    });
  });
});
