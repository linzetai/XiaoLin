import { describe, it, expect, vi, beforeEach } from "vitest";

vi.mock("../ws-client", () => ({
  connect: vi.fn(() => Promise.resolve()),
  disconnect: vi.fn(),
  send: vi.fn(() => Promise.resolve({})),
  on: vi.fn(() => vi.fn()),
  isConnected: vi.fn(() => false),
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(() => Promise.reject(new Error("not in tauri"))),
}));

import * as transport from "../transport";
import * as wsClient from "../ws-client";

const mockSend = wsClient.send as ReturnType<typeof vi.fn>;
const mockOn = wsClient.on as ReturnType<typeof vi.fn>;
const mockConnect = wsClient.connect as ReturnType<typeof vi.fn>;
const mockDisconnect = wsClient.disconnect as ReturnType<typeof vi.fn>;
const mockIsConnected = wsClient.isConnected as ReturnType<typeof vi.fn>;

describe("transport layer (browser mode)", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  // ═══════════════════════════════════════════════════════════════════
  // Environment detection
  // ═══════════════════════════════════════════════════════════════════

  describe("isTauri detection", () => {
    it("isTauri should be false in test environment", () => {
      expect(transport.isTauri).toBe(false);
    });
  });

  // ═══════════════════════════════════════════════════════════════════
  // listAgents (browser mode → WS)
  // ═══════════════════════════════════════════════════════════════════

  describe("listAgents", () => {
    it("calls wsClient.send with 'agents' method", async () => {
      mockSend.mockResolvedValueOnce({
        data: {
          agents: [
            { agentId: "main", name: "Main Agent", model: "gpt-4" },
            { agentId: "code", name: "Code Agent", model: "gpt-4" },
          ],
        },
      });
      const result = await transport.listAgents();
      expect(mockSend).toHaveBeenCalledWith("agents");
      expect(result).toHaveLength(2);
      expect(result[0].agentId).toBe("main");
      expect(result[1].name).toBe("Code Agent");
    });

    it("returns empty array when response has no agents", async () => {
      mockSend.mockResolvedValueOnce({});
      const result = await transport.listAgents();
      expect(result).toEqual([]);
    });

    it("returns empty array when response data is null", async () => {
      mockSend.mockResolvedValueOnce({ data: null });
      const result = await transport.listAgents();
      expect(result).toEqual([]);
    });
  });

  // ═══════════════════════════════════════════════════════════════════
  // listSessions
  // ═══════════════════════════════════════════════════════════════════

  describe("listSessions", () => {
    it("calls wsClient.send with correct params", async () => {
      mockSend.mockResolvedValueOnce({
        data: {
          sessions: [
            { id: "s1", agentId: "main", title: "Hello", messageCount: 5, createdAt: "2024-01-01", updatedAt: "2024-01-02" },
          ],
        },
      });
      const result = await transport.listSessions(10, 5);
      expect(mockSend).toHaveBeenCalledWith("sessions.list", { limit: 10, offset: 5 });
      expect(result).toHaveLength(1);
      expect(result[0].id).toBe("s1");
    });

    it("uses default limit and offset", async () => {
      mockSend.mockResolvedValueOnce({ data: { sessions: [] } });
      await transport.listSessions();
      expect(mockSend).toHaveBeenCalledWith("sessions.list", { limit: 50, offset: 0 });
    });

    it("returns empty array on no sessions", async () => {
      mockSend.mockResolvedValueOnce({});
      const result = await transport.listSessions();
      expect(result).toEqual([]);
    });
  });

  // ═══════════════════════════════════════════════════════════════════
  // getSession
  // ═══════════════════════════════════════════════════════════════════

  describe("getSession", () => {
    it("returns session data", async () => {
      const session = { id: "s1", agentId: "main", title: "Test", messageCount: 3, createdAt: "2024-01-01", updatedAt: "2024-01-01" };
      mockSend.mockResolvedValueOnce({ data: session });
      const result = await transport.getSession("s1");
      expect(mockSend).toHaveBeenCalledWith("sessions.get", { sessionId: "s1" });
      expect(result).toEqual(session);
    });

    it("returns null when session not found", async () => {
      mockSend.mockResolvedValueOnce({});
      const result = await transport.getSession("nonexistent");
      expect(result).toBeNull();
    });
  });

  // ═══════════════════════════════════════════════════════════════════
  // getSessionMessages
  // ═══════════════════════════════════════════════════════════════════

  describe("getSessionMessages", () => {
    it("returns messages with hasMore flag", async () => {
      const messages = [
        { id: 1, role: "user", content: "hello", name: null, toolCallId: null, createdAt: "2024-01-01" },
        { id: 2, role: "assistant", content: "hi", name: null, toolCallId: null, createdAt: "2024-01-01" },
      ];
      mockSend.mockResolvedValueOnce({ data: { messages, hasMore: true } });
      const result = await transport.getSessionMessages("s1");
      expect(mockSend).toHaveBeenCalledWith("sessions.messages", {
        sessionId: "s1",
        beforeId: undefined,
        limit: 30,
      });
      expect(result.messages).toHaveLength(2);
      expect(result.messages[0].role).toBe("user");
      expect(result.hasMore).toBe(true);
    });

    it("passes beforeId for cursor pagination", async () => {
      mockSend.mockResolvedValueOnce({ data: { messages: [], hasMore: false } });
      const result = await transport.getSessionMessages("s1", { beforeId: 42, limit: 10 });
      expect(mockSend).toHaveBeenCalledWith("sessions.messages", {
        sessionId: "s1",
        beforeId: 42,
        limit: 10,
      });
      expect(result.messages).toEqual([]);
      expect(result.hasMore).toBe(false);
    });

    it("returns empty page on no messages", async () => {
      mockSend.mockResolvedValueOnce({ data: {} });
      const result = await transport.getSessionMessages("s1");
      expect(result.messages).toEqual([]);
      expect(result.hasMore).toBe(false);
    });
  });

  // ═══════════════════════════════════════════════════════════════════
  // getToolOutput
  // ═══════════════════════════════════════════════════════════════════

  describe("getToolOutput", () => {
    it("fetches full output for a tool call", async () => {
      mockSend.mockResolvedValueOnce({
        data: { output: "full-output", displayOutput: "full-display", truncated: false },
      });
      const result = await transport.getToolOutput("s1", 10, "call-1");
      expect(mockSend).toHaveBeenCalledWith("sessions.tool_output", {
        sessionId: "s1",
        messageId: 10,
        callId: "call-1",
      });
      expect(result.output).toBe("full-output");
      expect(result.displayOutput).toBe("full-display");
    });

    it("returns empty object on missing data", async () => {
      mockSend.mockResolvedValueOnce({});
      const result = await transport.getToolOutput("s1", 10, "call-1");
      expect(result).toEqual({});
    });
  });

  // ═══════════════════════════════════════════════════════════════════
  // createSession
  // ═══════════════════════════════════════════════════════════════════

  describe("createSession", () => {
    it("creates with default agent", async () => {
      mockSend.mockResolvedValueOnce({ data: { sessionId: "new-123" } });
      const result = await transport.createSession();
      expect(mockSend).toHaveBeenCalledWith("sessions.new", {});
      expect(result).toBe("new-123");
    });

    it("creates with specific agent", async () => {
      mockSend.mockResolvedValueOnce({ data: { sessionId: "new-456" } });
      const result = await transport.createSession("code-agent");
      expect(mockSend).toHaveBeenCalledWith("sessions.new", { agentId: "code-agent" });
      expect(result).toBe("new-456");
    });

    it("returns empty string on failure", async () => {
      mockSend.mockResolvedValueOnce({});
      const result = await transport.createSession();
      expect(result).toBe("");
    });
  });

  // ═══════════════════════════════════════════════════════════════════
  // updateSessionTitle
  // ═══════════════════════════════════════════════════════════════════

  describe("updateSessionTitle", () => {
    it("calls wsClient.send with correct params", async () => {
      mockSend.mockResolvedValueOnce({});
      await transport.updateSessionTitle("s1", "New Title");
      expect(mockSend).toHaveBeenCalledWith("sessions.update_title", {
        sessionId: "s1",
        title: "New Title",
      });
    });
  });

  // ═══════════════════════════════════════════════════════════════════
  // deleteSession
  // ═══════════════════════════════════════════════════════════════════

  describe("deleteSession", () => {
    it("calls wsClient.send with session ID", async () => {
      mockSend.mockResolvedValueOnce({});
      await transport.deleteSession("s1");
      expect(mockSend).toHaveBeenCalledWith("sessions.delete", { sessionId: "s1" });
    });
  });

  // ═══════════════════════════════════════════════════════════════════
  // listModels
  // ═══════════════════════════════════════════════════════════════════

  describe("listModels", () => {
    it("returns model list from WS", async () => {
      mockSend.mockResolvedValueOnce({
        data: {
          models: [
            { agentId: "main", model: "gpt-4", provider: "openai", contextWindow: 128000, costPer1kInput: 0.03, costPer1kOutput: 0.06, supportsReasoning: false },
          ],
        },
      });
      const result = await transport.listModels();
      expect(mockSend).toHaveBeenCalledWith("models.list");
      expect(result).toHaveLength(1);
      expect(result[0].model).toBe("gpt-4");
    });

    it("returns empty array when no models", async () => {
      mockSend.mockResolvedValueOnce({});
      const result = await transport.listModels();
      expect(result).toEqual([]);
    });
  });

  // ═══════════════════════════════════════════════════════════════════
  // getConfig / setConfig
  // ═══════════════════════════════════════════════════════════════════

  describe("getConfig", () => {
    it("gets full config without key", async () => {
      mockSend.mockResolvedValueOnce({ data: { gateway: { port: 18789 } } });
      const result = await transport.getConfig();
      expect(mockSend).toHaveBeenCalledWith("config.get", {});
      expect(result).toEqual({ gateway: { port: 18789 } });
    });

    it("gets specific key", async () => {
      mockSend.mockResolvedValueOnce({ data: { key: "gateway.port", value: 18789 } });
      const result = await transport.getConfig("gateway.port");
      expect(mockSend).toHaveBeenCalledWith("config.get", { key: "gateway.port" });
      expect(result).toEqual({ key: "gateway.port", value: 18789 });
    });
  });

  describe("setConfig", () => {
    it("sends key and value", async () => {
      mockSend.mockResolvedValueOnce({ data: { persisted: true, pendingRestart: true } });
      const result = await transport.setConfig("logging.level", "debug");
      expect(mockSend).toHaveBeenCalledWith("config.set", { key: "logging.level", value: "debug" });
      expect(result.persisted).toBe(true);
      expect(result.pendingRestart).toBe(true);
    });

    it("handles failure response gracefully", async () => {
      mockSend.mockResolvedValueOnce({});
      const result = await transport.setConfig("x", "y");
      expect(result.persisted).toBe(false);
      expect(result.pendingRestart).toBe(false);
    });
  });

  // ═══════════════════════════════════════════════════════════════════
  // chatStream (browser mode → WS events)
  // ═══════════════════════════════════════════════════════════════════

  describe("chatStream (browser/WS mode)", () => {
    it("registers event handlers for all chat events", () => {
      mockSend.mockResolvedValueOnce({});
      const onEvent = vi.fn();
      transport.chatStream({ messages: [{ role: "user", content: "hi" }] }, onEvent);

      const registeredEvents = mockOn.mock.calls.map((c: unknown[]) => c[0]);
      expect(registeredEvents).toContain("turn_start");
      expect(registeredEvents).toContain("content_delta");
      expect(registeredEvents).toContain("turn_end");
      expect(registeredEvents).toContain("tool_executing");
      expect(registeredEvents).toContain("tool_result");
      expect(registeredEvents).toContain("error");
    });

    it("sends chat request via WS", () => {
      mockSend.mockResolvedValueOnce({});
      transport.chatStream(
        { messages: [{ role: "user", content: "hello" }], agentId: "main", sessionId: "s1" },
        vi.fn(),
      );
      expect(mockSend).toHaveBeenCalledWith("chat", expect.objectContaining({
        messages: [{ role: "user", content: "hello" }],
        agentId: "main",
        sessionId: "s1",
        stream: true,
      }));
    });

    it("includes workDir when provided", () => {
      mockSend.mockResolvedValueOnce({});
      transport.chatStream(
        { messages: [{ role: "user", content: "ls" }], workDir: "/home" },
        vi.fn(),
      );
      expect(mockSend).toHaveBeenCalledWith("chat", expect.objectContaining({
        workDir: "/home",
      }));
    });

    it("cleanup unsubscribes all event handlers", () => {
      const unsubFns = Array.from({ length: 6 }, () => vi.fn());
      let callIdx = 0;
      mockOn.mockImplementation(() => unsubFns[callIdx++]);
      mockSend.mockResolvedValueOnce({});

      const { cleanup } = transport.chatStream(
        { messages: [{ role: "user", content: "test" }] },
        vi.fn(),
      );

      cleanup();

      for (const fn of unsubFns) {
        expect(fn).toHaveBeenCalled();
      }
    });
  });

  // ═══════════════════════════════════════════════════════════════════
  // onSessionChanged (browser mode → WS subscribe)
  // ═══════════════════════════════════════════════════════════════════

  describe("onSessionChanged (browser mode)", () => {
    it("registers WS event listener and returns unsub function", () => {
      const unsubFn = vi.fn();
      mockOn.mockReturnValueOnce(unsubFn);

      const handler = vi.fn();
      const unsub = transport.onSessionChanged(handler);
      expect(mockOn).toHaveBeenCalledWith("sessions.changed", expect.any(Function));

      unsub();
      expect(unsubFn).toHaveBeenCalled();
    });
  });

  // ═══════════════════════════════════════════════════════════════════
  // WS passthrough functions
  // ═══════════════════════════════════════════════════════════════════

  describe("WS passthrough", () => {
    it("connectWs delegates to wsClient.connect", async () => {
      mockConnect.mockResolvedValueOnce(undefined);
      await transport.connectWs("ws://localhost:8080");
      expect(mockConnect).toHaveBeenCalledWith("ws://localhost:8080", undefined);
    });

    it("connectWs passes token", async () => {
      mockConnect.mockResolvedValueOnce(undefined);
      await transport.connectWs("ws://localhost:8080", "my-token");
      expect(mockConnect).toHaveBeenCalledWith("ws://localhost:8080", "my-token");
    });

    it("disconnectWs delegates to wsClient.disconnect", () => {
      transport.disconnectWs();
      expect(mockDisconnect).toHaveBeenCalled();
    });

    it("onWsEvent delegates to wsClient.on", () => {
      const handler = vi.fn();
      const unsub = vi.fn();
      mockOn.mockReturnValueOnce(unsub);
      const result = transport.onWsEvent("test-event", handler);
      expect(mockOn).toHaveBeenCalledWith("test-event", handler);
      expect(result).toBe(unsub);
    });

    it("isWsConnected delegates to wsClient.isConnected", () => {
      mockIsConnected.mockReturnValueOnce(true);
      expect(transport.isWsConnected()).toBe(true);
      mockIsConnected.mockReturnValueOnce(false);
      expect(transport.isWsConnected()).toBe(false);
    });
  });
});
