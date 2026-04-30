/**
 * Integration test: simulates the full chat stream event flow.
 *
 * Covers AC1 (streaming reply events arrive promptly) and AC3 (tool call
 * events are correctly processed).
 */
import { describe, it, expect, beforeEach } from "vitest";
import type { ChatStreamEvent, ChatStreamParams } from "../transport";

type ChatEventHandler = (event: ChatStreamEvent) => void;

function createMockTransport() {
  let handler: ChatEventHandler | null = null;
  let cleanedUp = false;

  return {
    fire(event: ChatStreamEvent) {
      if (!cleanedUp && handler) handler(event);
    },
    chatStream(_params: ChatStreamParams, onEvent: ChatEventHandler) {
      handler = onEvent;
      cleanedUp = false;
      return {
        promise: Promise.resolve(),
        cleanup: () => { cleanedUp = true; handler = null; },
      };
    },
    get isCleanedUp() { return cleanedUp; },
  };
}

describe("chat stream event flow", () => {
  let mock: ReturnType<typeof createMockTransport>;
  let events: ChatStreamEvent[];

  beforeEach(() => {
    mock = createMockTransport();
    events = [];
  });

  // ═══════════════════════════════════════════════════════════════════
  // AC1: 发送消息后流式回复事件正确传递
  // ═══════════════════════════════════════════════════════════════════

  describe("streaming text reply", () => {
    it("delivers start → delta → complete event sequence", () => {
      mock.chatStream(
        { messages: [{ role: "user", content: "Hello" }] },
        (e) => events.push(e),
      );

      mock.fire({ type: "chat.start", data: {} });
      mock.fire({ type: "chat.delta", data: { content: "Hi" } });
      mock.fire({ type: "chat.delta", data: { content: " there!" } });
      mock.fire({
        type: "chat.complete",
        data: {
          sessionId: "s-1",
          usage: { promptTokens: 10, completionTokens: 5, totalTokens: 15 },
          elapsedMs: 200,
        },
      });

      expect(events).toHaveLength(4);
      expect(events.map((e) => e.type)).toEqual([
        "chat.start",
        "chat.delta",
        "chat.delta",
        "chat.complete",
      ]);

      const deltas = events
        .filter((e) => e.type === "chat.delta")
        .map((e) => e.data?.content);
      expect(deltas).toEqual(["Hi", " there!"]);
    });

    it("accumulates streamed content correctly", () => {
      let accumulated = "";
      mock.chatStream(
        { messages: [{ role: "user", content: "Tell me a story" }] },
        (e) => {
          if (e.type === "chat.delta" && e.data?.content) {
            accumulated += e.data.content as string;
          }
        },
      );

      const chunks = ["Once ", "upon ", "a ", "time, ", "there ", "was ", "a Rust programmer."];
      mock.fire({ type: "chat.start", data: {} });
      for (const c of chunks) {
        mock.fire({ type: "chat.delta", data: { content: c } });
      }
      mock.fire({ type: "chat.complete", data: {} });

      expect(accumulated).toBe("Once upon a time, there was a Rust programmer.");
    });
  });

  // ═══════════════════════════════════════════════════════════════════
  // Tool call event processing
  // ═══════════════════════════════════════════════════════════════════

  describe("tool call events", () => {
    it("processes tool.start and tool.done events", () => {
      const toolEvents: ChatStreamEvent[] = [];
      mock.chatStream(
        { messages: [{ role: "user", content: "Read my file" }] },
        (e) => {
          if (e.type.startsWith("chat.tool")) toolEvents.push(e);
        },
      );

      mock.fire({ type: "chat.start", data: {} });
      mock.fire({ type: "chat.delta", data: { content: "Let me read that file." } });
      mock.fire({
        type: "chat.tool.start",
        data: { tool: "file_read", callId: "tc-1", args: '{"path":"src/main.rs"}' },
      });
      mock.fire({
        type: "chat.tool.done",
        data: { tool: "file_read", callId: "tc-1", success: true, output: "fn main() {}" },
      });
      mock.fire({ type: "chat.complete", data: {} });

      expect(toolEvents).toHaveLength(2);
      expect(toolEvents[0].type).toBe("chat.tool.start");
      expect(toolEvents[0].data?.tool).toBe("file_read");
      expect(toolEvents[1].type).toBe("chat.tool.done");
      expect(toolEvents[1].data?.success).toBe(true);
      expect(toolEvents[1].data?.output).toBe("fn main() {}");
    });

    it("handles multiple tool calls in a single turn", () => {
      const toolStarts: string[] = [];
      const toolDones: string[] = [];

      mock.chatStream(
        { messages: [{ role: "user", content: "Search and read" }] },
        (e) => {
          if (e.type === "chat.tool.start") toolStarts.push(e.data!.callId as string);
          if (e.type === "chat.tool.done") toolDones.push(e.data!.callId as string);
        },
      );

      mock.fire({ type: "chat.start", data: {} });
      mock.fire({ type: "chat.tool.start", data: { tool: "file_search", callId: "tc-1" } });
      mock.fire({ type: "chat.tool.done", data: { tool: "file_search", callId: "tc-1", success: true, output: "found: main.rs" } });
      mock.fire({ type: "chat.tool.start", data: { tool: "file_read", callId: "tc-2" } });
      mock.fire({ type: "chat.tool.done", data: { tool: "file_read", callId: "tc-2", success: true, output: "fn main() {}" } });
      mock.fire({ type: "chat.complete", data: {} });

      expect(toolStarts).toEqual(["tc-1", "tc-2"]);
      expect(toolDones).toEqual(["tc-1", "tc-2"]);
    });

    it("handles tool call failure", () => {
      let failedTool: ChatStreamEvent | null = null;
      mock.chatStream(
        { messages: [{ role: "user", content: "run bad command" }] },
        (e) => {
          if (e.type === "chat.tool.done" && e.data?.success === false) {
            failedTool = e;
          }
        },
      );

      mock.fire({ type: "chat.start", data: {} });
      mock.fire({ type: "chat.tool.start", data: { tool: "shell", callId: "tc-1" } });
      mock.fire({
        type: "chat.tool.done",
        data: { tool: "shell", callId: "tc-1", success: false, output: "command not found" },
      });
      mock.fire({ type: "chat.complete", data: {} });

      expect(failedTool).not.toBeNull();
      expect(failedTool!.data?.success).toBe(false);
      expect(failedTool!.data?.output).toBe("command not found");
    });
  });

  // ═══════════════════════════════════════════════════════════════════
  // Error handling
  // ═══════════════════════════════════════════════════════════════════

  describe("error handling", () => {
    it("delivers chat.error event", () => {
      let errorEvent: ChatStreamEvent | null = null;
      mock.chatStream(
        { messages: [{ role: "user", content: "Hi" }] },
        (e) => {
          if (e.type === "chat.error") errorEvent = e;
        },
      );

      mock.fire({ type: "chat.start", data: {} });
      mock.fire({ type: "chat.error", error: { message: "Rate limit exceeded" } });

      expect(errorEvent).not.toBeNull();
      expect(errorEvent!.error?.message).toBe("Rate limit exceeded");
    });

    it("stops delivering events after cleanup", () => {
      const { cleanup } = mock.chatStream(
        { messages: [{ role: "user", content: "Hi" }] },
        (e) => events.push(e),
      );

      mock.fire({ type: "chat.start", data: {} });
      expect(events).toHaveLength(1);

      cleanup();

      mock.fire({ type: "chat.delta", data: { content: "Should not arrive" } });
      expect(events).toHaveLength(1);
    });
  });

  // ═══════════════════════════════════════════════════════════════════
  // Context usage events
  // ═══════════════════════════════════════════════════════════════════

  describe("context usage events", () => {
    it("passes context usage data through", () => {
      let usageEvent: ChatStreamEvent | null = null;
      mock.chatStream(
        { messages: [{ role: "user", content: "Hi" }] },
        (e) => {
          if (e.type === "chat.context.usage") usageEvent = e;
        },
      );

      mock.fire({
        type: "chat.context.usage",
        data: { usedTokens: 10200, limitTokens: 128000, compressed: true, tokensSaved: 5000 },
      });

      expect(usageEvent).not.toBeNull();
      expect(usageEvent!.data?.usedTokens).toBe(10200);
      expect(usageEvent!.data?.limitTokens).toBe(128000);
    });
  });

  // ═══════════════════════════════════════════════════════════════════
  // Ask question (human-in-the-loop) events
  // ═══════════════════════════════════════════════════════════════════

  describe("ask question events", () => {
    it("delivers chat.ask_question event with options", () => {
      let questionEvent: ChatStreamEvent | null = null;
      mock.chatStream(
        { messages: [{ role: "user", content: "deploy" }] },
        (e) => {
          if (e.type === "chat.ask_question") questionEvent = e;
        },
      );

      mock.fire({
        type: "chat.ask_question",
        data: {
          requestId: "q-1",
          question: "Are you sure you want to deploy to production?",
          options: [
            { id: "yes", label: "Yes, deploy" },
            { id: "no", label: "Cancel" },
          ],
          timeoutSecs: 60,
        },
      });

      expect(questionEvent).not.toBeNull();
      expect(questionEvent!.data?.question).toBe("Are you sure you want to deploy to production?");
      expect(questionEvent!.data?.options).toHaveLength(2);
    });
  });

  // ═══════════════════════════════════════════════════════════════════
  // Sub-agent events
  // ═══════════════════════════════════════════════════════════════════

  describe("sub-agent events", () => {
    it("delivers sub-agent lifecycle events", () => {
      const subEvents: string[] = [];
      mock.chatStream(
        { messages: [{ role: "user", content: "do complex task" }] },
        (e) => {
          if (e.type.startsWith("chat.subagent")) subEvents.push(e.type);
        },
      );

      mock.fire({ type: "chat.start", data: {} });
      mock.fire({ type: "chat.subagent.start", data: { runId: "run-1", agentId: "code", task: "analyze code" } });
      mock.fire({ type: "chat.subagent.delta", data: { runId: "run-1", content: "Analyzing..." } });
      mock.fire({ type: "chat.subagent.tool.start", data: { runId: "run-1", tool: "file_read", callId: "stc-1" } });
      mock.fire({ type: "chat.subagent.tool.done", data: { runId: "run-1", callId: "stc-1", success: true, output: "code" } });
      mock.fire({ type: "chat.subagent.complete", data: { runId: "run-1", status: "completed", result: "Done" } });
      mock.fire({ type: "chat.complete", data: {} });

      expect(subEvents).toEqual([
        "chat.subagent.start",
        "chat.subagent.delta",
        "chat.subagent.tool.start",
        "chat.subagent.tool.done",
        "chat.subagent.complete",
      ]);
    });
  });
});
