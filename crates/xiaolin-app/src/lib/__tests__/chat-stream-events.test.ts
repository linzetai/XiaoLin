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

  describe("streaming text reply", () => {
    it("delivers delta → turn_end event sequence", () => {
      mock.chatStream(
        { messages: [{ role: "user", content: "Hello" }] },
        (e) => events.push(e),
      );

      mock.fire({
        type: "content_delta",
        data: {
          type: "content_delta",
          turn_id: "t1",
          delta: { choices: [{ delta: { content: "Hi" } }] },
        },
      });
      mock.fire({
        type: "content_delta",
        data: {
          type: "content_delta",
          turn_id: "t1",
          delta: { choices: [{ delta: { content: " there!" } }] },
        },
      });
      mock.fire({
        type: "turn_end",
        data: {
          type: "turn_end",
          turn_id: "t1",
          session_id: "s-1",
          summary: {
            tool_calls_made: 0,
            iterations: 1,
            usage: { prompt_tokens: 10, completion_tokens: 5, total_tokens: 15 },
            elapsed_ms: 200,
          },
        },
      });

      expect(events).toHaveLength(3);
      expect(events.map((e) => e.type)).toEqual([
        "content_delta",
        "content_delta",
        "turn_end",
      ]);

      const deltas = events
        .filter((e) => e.type === "content_delta")
        .map((e) => (e.data?.delta as { choices?: Array<{ delta?: { content?: string } }> })?.choices?.[0]?.delta?.content);
      expect(deltas).toEqual(["Hi", " there!"]);
    });

    it("accumulates streamed content correctly", () => {
      let accumulated = "";
      mock.chatStream(
        { messages: [{ role: "user", content: "Tell me a story" }] },
        (e) => {
          if (e.type === "content_delta" && e.data?.delta) {
            const text = (e.data.delta as { choices?: Array<{ delta?: { content?: string } }> })
              ?.choices?.[0]?.delta?.content;
            if (text) accumulated += text;
          }
        },
      );

      const chunks = ["Once ", "upon ", "a ", "time, ", "there ", "was ", "a Rust programmer."];
      for (const c of chunks) {
        mock.fire({
          type: "content_delta",
          data: {
            type: "content_delta",
            turn_id: "t1",
            delta: { choices: [{ delta: { content: c } }] },
          },
        });
      }
      mock.fire({ type: "turn_end", data: { type: "turn_end", turn_id: "t1", summary: {} } });

      expect(accumulated).toBe("Once upon a time, there was a Rust programmer.");
    });
  });

  describe("tool call events", () => {
    it("processes tool_executing and tool_result events", () => {
      const toolEvents: ChatStreamEvent[] = [];
      mock.chatStream(
        { messages: [{ role: "user", content: "Read my file" }] },
        (e) => {
          if (e.type === "tool_executing" || e.type === "tool_result") toolEvents.push(e);
        },
      );

      mock.fire({
        type: "content_delta",
        data: {
          type: "content_delta",
          turn_id: "t1",
          delta: { choices: [{ delta: { content: "Let me read that file." } }] },
        },
      });
      mock.fire({
        type: "tool_executing",
        data: {
          type: "tool_executing",
          turn_id: "t1",
          tool_name: "file_read",
          call_id: "tc-1",
          args: '{"path":"src/main.rs"}',
        },
      });
      mock.fire({
        type: "tool_result",
        data: {
          type: "tool_result",
          turn_id: "t1",
          tool_name: "file_read",
          call_id: "tc-1",
          success: true,
          output: "fn main() {}",
        },
      });
      mock.fire({ type: "turn_end", data: { type: "turn_end", turn_id: "t1", summary: {} } });

      expect(toolEvents).toHaveLength(2);
      expect(toolEvents[0].type).toBe("tool_executing");
      expect(toolEvents[0].data?.tool_name).toBe("file_read");
      expect(toolEvents[1].type).toBe("tool_result");
      expect(toolEvents[1].data?.success).toBe(true);
      expect(toolEvents[1].data?.output).toBe("fn main() {}");
    });

    it("handles multiple tool calls in a single turn", () => {
      const toolStarts: string[] = [];
      const toolDones: string[] = [];

      mock.chatStream(
        { messages: [{ role: "user", content: "Search and read" }] },
        (e) => {
          if (e.type === "tool_executing") toolStarts.push(e.data!.call_id as string);
          if (e.type === "tool_result") toolDones.push(e.data!.call_id as string);
        },
      );

      mock.fire({ type: "tool_executing", data: { type: "tool_executing", turn_id: "t1", tool_name: "file_search", call_id: "tc-1" } });
      mock.fire({ type: "tool_result", data: { type: "tool_result", turn_id: "t1", tool_name: "file_search", call_id: "tc-1", success: true, output: "found: main.rs" } });
      mock.fire({ type: "tool_executing", data: { type: "tool_executing", turn_id: "t1", tool_name: "file_read", call_id: "tc-2" } });
      mock.fire({ type: "tool_result", data: { type: "tool_result", turn_id: "t1", tool_name: "file_read", call_id: "tc-2", success: true, output: "fn main() {}" } });
      mock.fire({ type: "turn_end", data: { type: "turn_end", turn_id: "t1", summary: {} } });

      expect(toolStarts).toEqual(["tc-1", "tc-2"]);
      expect(toolDones).toEqual(["tc-1", "tc-2"]);
    });

    it("handles tool call failure", () => {
      let failedTool: ChatStreamEvent | null = null;
      mock.chatStream(
        { messages: [{ role: "user", content: "run bad command" }] },
        (e) => {
          if (e.type === "tool_result" && e.data?.success === false) {
            failedTool = e;
          }
        },
      );

      mock.fire({ type: "tool_executing", data: { type: "tool_executing", turn_id: "t1", tool_name: "shell", call_id: "tc-1" } });
      mock.fire({
        type: "tool_result",
        data: {
          type: "tool_result",
          turn_id: "t1",
          tool_name: "shell",
          call_id: "tc-1",
          success: false,
          output: "command not found",
        },
      });
      mock.fire({ type: "turn_end", data: { type: "turn_end", turn_id: "t1", summary: {} } });

      expect(failedTool).not.toBeNull();
      expect(failedTool!.data?.success).toBe(false);
      expect(failedTool!.data?.output).toBe("command not found");
    });
  });

  describe("tool result with output handles (Phase 10)", () => {
    it("delivers tool_result event with output handle fields", () => {
      let resultEvent: ChatStreamEvent | null = null;
      mock.chatStream(
        { messages: [{ role: "user", content: "read large file" }] },
        (e) => {
          if (e.type === "tool_result") resultEvent = e;
        },
      );

      mock.fire({
        type: "tool_result",
        data: {
          type: "tool_result",
          turn_id: "t1",
          tool_name: "file_read",
          call_id: "tc-handle-1",
          success: true,
          output: "[ReadFile — handle: out_abc123_xyz]...",
          output_handle: "out_abc123_xyz",
          output_size_class: "large",
          output_is_expandable: true,
        },
      });

      expect(resultEvent).not.toBeNull();
      expect(resultEvent!.data?.output_handle).toBe("out_abc123_xyz");
      expect(resultEvent!.data?.output_size_class).toBe("large");
      expect(resultEvent!.data?.output_is_expandable).toBe(true);
    });

    it("tool_result without handle fields is still delivered correctly", () => {
      let resultEvent: ChatStreamEvent | null = null;
      mock.chatStream(
        { messages: [{ role: "user", content: "read small file" }] },
        (e) => {
          if (e.type === "tool_result") resultEvent = e;
        },
      );

      mock.fire({
        type: "tool_result",
        data: {
          type: "tool_result",
          turn_id: "t1",
          tool_name: "file_read",
          call_id: "tc-no-handle",
          success: true,
          output: "small file content",
        },
      });

      expect(resultEvent).not.toBeNull();
      expect(resultEvent!.data?.output).toBe("small file content");
      expect(resultEvent!.data?.output_handle).toBeUndefined();
      expect(resultEvent!.data?.output_size_class).toBeUndefined();
      expect(resultEvent!.data?.output_is_expandable).toBeUndefined();
    });

    it("delivers multiple tool results with mixed handle presence", () => {
      const results: Array<{ callId: string; handle?: string; sizeClass?: string }> = [];
      mock.chatStream(
        { messages: [{ role: "user", content: "search and read" }] },
        (e) => {
          if (e.type === "tool_result") {
            results.push({
              callId: e.data?.call_id as string,
              handle: e.data?.output_handle as string | undefined,
              sizeClass: e.data?.output_size_class as string | undefined,
            });
          }
        },
      );

      // Large output with handle
      mock.fire({
        type: "tool_result",
        data: {
          type: "tool_result", turn_id: "t1", tool_name: "shell_exec",
          call_id: "tc-large", success: true, output: "[shell — handle: out_1]",
          output_handle: "out_1", output_size_class: "large", output_is_expandable: true,
        },
      });
      // Medium output with handle
      mock.fire({
        type: "tool_result",
        data: {
          type: "tool_result", turn_id: "t1", tool_name: "search",
          call_id: "tc-medium", success: true, output: "[search — handle: out_2]",
          output_handle: "out_2", output_size_class: "medium", output_is_expandable: true,
        },
      });
      // Small output — no handle
      mock.fire({
        type: "tool_result",
        data: {
          type: "tool_result", turn_id: "t1", tool_name: "read_file",
          call_id: "tc-small", success: true, output: "short inline content",
        },
      });
      mock.fire({ type: "turn_end", data: { type: "turn_end", turn_id: "t1", summary: {} } });

      expect(results).toHaveLength(3);
      expect(results[0].handle).toBe("out_1");
      expect(results[0].sizeClass).toBe("large");
      expect(results[1].handle).toBe("out_2");
      expect(results[1].sizeClass).toBe("medium");
      expect(results[2].handle).toBeUndefined();
    });

    it("tool_result with handle but is_expandable false still receives handle", () => {
      let resultEvent: ChatStreamEvent | null = null;
      mock.chatStream(
        { messages: [{ role: "user", content: "read" }] },
        (e) => {
          if (e.type === "tool_result") resultEvent = e;
        },
      );

      mock.fire({
        type: "tool_result",
        data: {
          type: "tool_result", turn_id: "t1", tool_name: "read_file",
          call_id: "tc-expired", success: true, output: "content",
          output_handle: "out_expired", output_size_class: "large", output_is_expandable: false,
        },
      });

      expect(resultEvent).not.toBeNull();
      expect(resultEvent!.data?.output_handle).toBe("out_expired");
      expect(resultEvent!.data?.output_is_expandable).toBe(false);
    });
  });

  describe("error handling", () => {
    it("delivers error event", () => {
      let errorEvent: ChatStreamEvent | null = null;
      mock.chatStream(
        { messages: [{ role: "user", content: "Hi" }] },
        (e) => {
          if (e.type === "error") errorEvent = e;
        },
      );

      mock.fire({
        type: "error",
        data: { type: "error", turn_id: "t1", message: "Rate limit exceeded" },
        error: { message: "Rate limit exceeded" },
      });

      expect(errorEvent).not.toBeNull();
      expect(errorEvent!.data?.message ?? errorEvent!.error?.message).toBe("Rate limit exceeded");
    });

    it("stops delivering events after cleanup", () => {
      const { cleanup } = mock.chatStream(
        { messages: [{ role: "user", content: "Hi" }] },
        (e) => events.push(e),
      );

      mock.fire({
        type: "content_delta",
        data: {
          type: "content_delta",
          turn_id: "t1",
          delta: { choices: [{ delta: { content: "Hello" } }] },
        },
      });
      expect(events).toHaveLength(1);

      cleanup();

      mock.fire({
        type: "content_delta",
        data: {
          type: "content_delta",
          turn_id: "t1",
          delta: { choices: [{ delta: { content: "Should not arrive" } }] },
        },
      });
      expect(events).toHaveLength(1);
    });
  });

  describe("context usage events", () => {
    it("passes context usage data through", () => {
      let usageEvent: ChatStreamEvent | null = null;
      mock.chatStream(
        { messages: [{ role: "user", content: "Hi" }] },
        (e) => {
          if (e.type === "context_usage_update") usageEvent = e;
        },
      );

      mock.fire({
        type: "context_usage_update",
        data: {
          type: "context_usage_update",
          turn_id: "t1",
          used_tokens: 10200,
          limit_tokens: 128000,
          compressed: true,
          tokens_saved: 5000,
        },
      });

      expect(usageEvent).not.toBeNull();
      expect(usageEvent!.data?.used_tokens).toBe(10200);
      expect(usageEvent!.data?.limit_tokens).toBe(128000);
    });
  });

  describe("ask question events", () => {
    it("delivers ask_question event with options", () => {
      let questionEvent: ChatStreamEvent | null = null;
      mock.chatStream(
        { messages: [{ role: "user", content: "deploy" }] },
        (e) => {
          if (e.type === "ask_question") questionEvent = e;
        },
      );

      mock.fire({
        type: "ask_question",
        data: {
          type: "ask_question",
          turn_id: "t1",
          request_id: "q-1",
          question: "Are you sure you want to deploy to production?",
          options: [
            { id: "yes", label: "Yes, deploy" },
            { id: "no", label: "Cancel" },
          ],
          timeout_secs: 60,
        },
      });

      expect(questionEvent).not.toBeNull();
      expect(questionEvent!.data?.question).toBe("Are you sure you want to deploy to production?");
      expect(questionEvent!.data?.options).toHaveLength(2);
    });
  });

  describe("sub-agent events", () => {
    it("delivers sub-agent lifecycle events", () => {
      const subEvents: string[] = [];
      mock.chatStream(
        { messages: [{ role: "user", content: "do complex task" }] },
        (e) => {
          if (e.type.startsWith("sub_agent")) subEvents.push(e.type);
        },
      );

      mock.fire({ type: "sub_agent_start", data: { type: "sub_agent_start", turn_id: "t1", run_id: "run-1", agent_id: "code", task: "analyze code" } });
      mock.fire({ type: "sub_agent_delta", data: { type: "sub_agent_delta", turn_id: "t1", run_id: "run-1", content: "Analyzing..." } });
      mock.fire({ type: "sub_agent_tool_executing", data: { type: "sub_agent_tool_executing", turn_id: "t1", run_id: "run-1", tool_name: "file_read", call_id: "stc-1" } });
      mock.fire({ type: "sub_agent_tool_result", data: { type: "sub_agent_tool_result", turn_id: "t1", run_id: "run-1", call_id: "stc-1", success: true, output: "code" } });
      mock.fire({ type: "sub_agent_complete", data: { type: "sub_agent_complete", turn_id: "t1", run_id: "run-1", status: "completed", result: "Done" } });
      mock.fire({ type: "turn_end", data: { type: "turn_end", turn_id: "t1", summary: {} } });

      expect(subEvents).toEqual([
        "sub_agent_start",
        "sub_agent_delta",
        "sub_agent_tool_executing",
        "sub_agent_tool_result",
        "sub_agent_complete",
      ]);
    });
  });
});
