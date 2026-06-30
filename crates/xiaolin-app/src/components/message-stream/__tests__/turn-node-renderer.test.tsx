/**
 * @vitest-environment jsdom
 *
 * TurnNodeRenderer and Phase 5 rendering tests.
 *
 * Covers:
 * - Node-type → correct component mapping
 * - Text node streaming vs completed states
 * - Reasoning node active vs completed states
 * - Iteration boundary rendering
 * - Terminal status rendering (tool_loop, cancellation, abort, budget)
 * - System notice rendering
 * - Replay hydration equivalence (reducer → renderer node content)
 * - TurnNodeRenderer with empty nodes array
 */

import { beforeEach, describe, it, expect, vi } from "vitest";
import { fireEvent, render, waitFor } from "@testing-library/react";
import { TurnNodeRenderer } from "../TurnNodeRenderer";
import { TurnBlock } from "../TurnBlock";
import * as api from "../../../lib/api";
import {
  reduceTimelineEvents,
  materializeNodes,
  emptyTimelineState,
  reduceTimelineEvent,
  selectTurnGroups,
} from "../../../lib/timeline";
import type { TurnGroup } from "../../../lib/timeline/selectors";
import {
  simpleTextTurnFixture,
  complexTurnFixture,
  toolLoopTerminationFixture,
  makeTextDelta,
  makeToolStarted,
  makeToolFinished,
  makeReasoningDelta,
  makeUserMessageCreated,
  makeTurnStarted,
  makeTurnFinished,
} from "../../../lib/timeline/fixtures";
import type {
  AssistantTextNode,
  ReasoningNode,
  IterationBoundaryNode,
  TurnStatusNode,
  SystemNoticeNode,
  ApprovalNode,
  ToolGroupNode,
  ToolStepNode,
} from "../../../lib/timeline/types";

vi.mock("../../../lib/api", () => ({
  getToolOutputDetail: vi.fn(() => Promise.resolve({
    content: "line 1\nline 2",
    truncated: false,
    total_bytes: 13,
    total_lines: 2,
  })),
}));

beforeEach(() => {
  vi.mocked(api.getToolOutputDetail).mockReset();
  vi.mocked(api.getToolOutputDetail).mockResolvedValue({
    content: "line 1\nline 2",
    truncated: false,
    total_bytes: 13,
    total_lines: 2,
  });
});

function toolStep(overrides: Partial<ToolStepNode> = {}): ToolStepNode {
  return {
    kind: "tool_step",
    node_id: "tool-default",
    turn_id: "t1",
    status: "completed",
    created_at_ms: 1000,
    updated_at_ms: 1200,
    tool_name: "read_file",
    tool_category: "file",
    display_title: "Read README.md",
    call_id: "tc-default",
    ...overrides,
  };
}

function smallPreview(
  content: string,
  contentType = "text",
): NonNullable<ToolStepNode["output_preview"]> {
  return {
    content,
    byte_length: new TextEncoder().encode(content).byteLength,
    line_count: content.split("\n").length,
    estimated_tokens: Math.max(1, Math.ceil(content.length / 4)),
    is_binary: false,
    content_type: contentType,
  };
}

// ============================================================================
// Smoke tests — renderer doesn't crash
// ============================================================================

describe("TurnNodeRenderer smoke tests", () => {
  it("renders empty nodes array without crashing", () => {
    const { container } = render(<TurnNodeRenderer nodes={[]} />);
    expect(container.innerHTML).toBe("");
  });

  it("does not render whitespace-only assistant text or reasoning placeholders", () => {
    const blankText: AssistantTextNode = {
      kind: "assistant_text",
      node_id: "blank-text",
      turn_id: "t1",
      status: "pending",
      created_at_ms: 1000,
      updated_at_ms: 1000,
      content: "\n  ",
      text_role: "activity",
    };
    const blankReasoning: ReasoningNode = {
      kind: "reasoning",
      node_id: "blank-reasoning",
      turn_id: "t1",
      status: "pending",
      created_at_ms: 1000,
      updated_at_ms: 1000,
      content: "\n",
      collapsed: false,
      visibility: "public",
    };

    const { container } = render(
      <TurnNodeRenderer nodes={[blankText, blankReasoning]} isLive />,
    );

    expect(container.innerHTML).toBe("");
  });

  it("renders only one live cursor for multiple pending text nodes", () => {
    const first: AssistantTextNode = {
      kind: "assistant_text",
      node_id: "text-1",
      turn_id: "t1",
      status: "pending",
      created_at_ms: 1000,
      updated_at_ms: 1000,
      content: "First pending segment.",
      text_role: "final",
    };
    const second: AssistantTextNode = {
      ...first,
      node_id: "text-2",
      updated_at_ms: 1001,
      content: "Second pending segment.",
    };

    const { container } = render(
      <TurnNodeRenderer nodes={[first, second]} isLive />,
    );

    expect(container.querySelectorAll("[data-streaming-cursor]")).toHaveLength(1);
  });

  it("shows thinking state for a live turn before assistant nodes arrive", () => {
    const turnGroup: TurnGroup = {
      groupId: "t-thinking:0",
      turnId: "t-thinking",
      userMessageNode: {
        kind: "user_message",
        node_id: "u-thinking",
        turn_id: "t-thinking",
        status: "pending",
        created_at_ms: 1000,
        updated_at_ms: 1000,
        content: "review下代码",
        attachments: [],
      },
      assistantNodes: [],
    };

    const { container } = render(<TurnBlock turnGroup={turnGroup} isLive />);

    expect(container.textContent).toContain("review下代码");
    expect(container.textContent).not.toBe("review下代码");
  });

  it("renders text-only turn nodes without crashing", async () => {
    const events = simpleTextTurnFixture();
    const state = reduceTimelineEvents(events);
    const { container } = render(
      <TurnNodeRenderer nodes={state.nodes} />,
    );
    // User message renders immediately (not lazy)
    expect(container.textContent).toContain("What is 2+2?");
    // Assistant text renders via lazy MarkdownContent; wait for it
    await waitFor(() => {
      expect(container.textContent).toContain("2+2 = 4");
    }, { timeout: 5000 });
  });

  it("renders complex turn nodes without crashing", () => {
    const events = complexTurnFixture();
    const state = reduceTimelineEvents(events);
    const { container } = render(
      <TurnNodeRenderer nodes={state.nodes} />,
    );
    expect(container.innerHTML.length).toBeGreaterThan(0);
  });

  it("renders tool loop termination without crashing", () => {
    const events = toolLoopTerminationFixture();
    const state = reduceTimelineEvents(events);
    const { container } = render(
      <TurnNodeRenderer nodes={state.nodes} />,
    );
    expect(container.innerHTML.length).toBeGreaterThan(0);
  });
});

// ============================================================================
// Node-type → correct view mapping (via rendered output)
// ============================================================================

describe("Node view mapping", () => {
  it("renders assistant text, tool step, and resumed text in timeline order", async () => {
    const events = [
      makeTextDelta({
        seq: 1,
        payload: { node_id: "text-before", delta: "I will inspect the file first." },
      }),
      makeToolStarted({
        seq: 2,
        payload: {
          call_id: "tc-read-order",
          tool_name: "read_file",
          tool_category: "file",
          display_title: "Read README.md",
          args: '{"path":"README.md"}',
        },
      }),
      makeToolFinished({
        seq: 3,
        payload: {
          call_id: "tc-read-order",
          tool_name: "read_file",
          success: true,
          output_preview: { ...smallPreview("contents") },
        },
      }),
      makeTextDelta({
        seq: 4,
        payload: { node_id: "text-after", delta: "The file is straightforward." },
      }),
    ];
    const nodes = reduceTimelineEvents(events).nodes;

    const { container } = render(
      <TurnNodeRenderer nodes={nodes} sessionId="session-1" />,
    );

    await waitFor(() => {
      expect(container.textContent).toContain("I will inspect the file first.");
      expect(container.textContent).toContain("已读取");
      expect(container.textContent).toContain("README.md");
      expect(container.textContent).toContain("The file is straightforward.");
    });

    const text = container.textContent ?? "";
    expect(text.indexOf("I will inspect the file first.")).toBeLessThan(
      text.indexOf("已读取"),
    );
    expect(text.indexOf("已读取")).toBeLessThan(
      text.indexOf("The file is straightforward."),
    );
  });

  it("assistant_text node renders content in markdown", async () => {
    const events = simpleTextTurnFixture();
    const state = reduceTimelineEvents(events);
    const textNode = state.nodes.find(
      (n) => n.kind === "assistant_text",
    ) as AssistantTextNode;
    expect(textNode).toBeDefined();

    const { container } = render(
      <TurnNodeRenderer nodes={[textNode]} />,
    );
    await waitFor(() => {
      expect(container.textContent).toContain("2+2 = 4");
    }, { timeout: 5000 });
  });

  it("does not render raw reasoning nodes", () => {
    const node: ReasoningNode = {
      kind: "reasoning",
      node_id: "r-1",
      turn_id: "t1",
      status: "completed",
      created_at_ms: 1000,
      updated_at_ms: 2000,
      content: "Let me think about this.",
      collapsed: true,
      visibility: "public",
    };

    const { container } = render(
      <TurnNodeRenderer nodes={[node]} />,
    );
    expect(container.textContent).not.toContain("Let me think about this.");
    expect(container.querySelector('[data-timeline-node-kind="reasoning"]')).toBeNull();
  });

  it("iteration_boundary node renders a quiet divider without iteration text", () => {
    const node: IterationBoundaryNode = {
      kind: "iteration_boundary",
      node_id: "ib-1",
      turn_id: "t1",
      status: "completed",
      created_at_ms: 1000,
      updated_at_ms: 1000,
      iteration: 3,
    };

    const { container } = render(
      <TurnNodeRenderer nodes={[node]} />,
    );
    expect(container.textContent).not.toContain("iteration");
    expect(container.innerHTML).toBe("");
  });

  it("turn_status node shows tool_loop diagnosis", () => {
    const node: TurnStatusNode = {
      kind: "turn_status",
      node_id: "ts-1",
      turn_id: "t1",
      status: "failed",
      created_at_ms: 1000,
      updated_at_ms: 1000,
      end_reason: "tool_loop",
      summary: "Turn stopped by tool loop protection.",
      diagnosis: {
        diagnosis_code: "tool_loop",
        severity: "error",
        iterations: 12,
        tool_calls: 45,
      },
    };

    const { container } = render(
      <TurnNodeRenderer nodes={[node]} />,
    );
    expect(container.textContent).toContain(
      "Turn stopped by tool loop protection.",
    );
    expect(container.textContent).toContain("Code：tool_loop");
  });

  it("turn_status node for normal completion renders nothing", () => {
    const node: TurnStatusNode = {
      kind: "turn_status",
      node_id: "ts-1",
      turn_id: "t1",
      status: "completed",
      created_at_ms: 1000,
      updated_at_ms: 1000,
      end_reason: "completed",
    };

    const { container } = render(
      <TurnNodeRenderer nodes={[node]} />,
    );
    expect(container.innerHTML).toBe("");
  });

  it("system_notice node renders message", () => {
    const node: SystemNoticeNode = {
      kind: "system_notice",
      node_id: "sn-1",
      turn_id: "t1",
      status: "completed",
      created_at_ms: 1000,
      updated_at_ms: 1000,
      message: "Context was compacted.",
      level: "info",
      category: "compaction",
    };

    const { container } = render(
      <TurnNodeRenderer nodes={[node]} />,
    );
    expect(container.textContent).toContain("Context was compacted.");
  });

  it("approval node renders resolution", () => {
    const node: ApprovalNode = {
      kind: "approval",
      node_id: "ap-1",
      turn_id: "t1",
      status: "completed",
      created_at_ms: 1000,
      updated_at_ms: 2000,
      approval_id: "apr-1",
      action: "execute_command",
      reason: "Running cargo build",
      decision: "allow_once",
      decision_source: "user",
    };

    const { container } = render(
      <TurnNodeRenderer nodes={[node]} />,
    );
    expect(container.textContent).toContain("已批准执行命令");
    expect(container.textContent).toContain("Running cargo build");
    fireEvent.click(container.querySelector("button")!);
    expect(container.textContent).toContain("决策：allow_once");
    expect(container.textContent).toContain("来源：user");
  });

  it("tool_step renders compact metadata and small output without detail fetch", () => {
    const node: ToolStepNode = {
      kind: "tool_step",
      node_id: "tool-small",
      turn_id: "t1",
      status: "completed",
      created_at_ms: 1000,
      updated_at_ms: 1200,
      tool_name: "read_file",
      tool_category: "file",
      display_title: "Read README.md",
      call_id: "tc-small",
      target: { path: "README.md" },
      duration_ms: 200,
      output_preview: {
        content: "hello\nworld",
        byte_length: 11,
        line_count: 2,
        estimated_tokens: 3,
        is_binary: false,
        content_type: "text",
      },
    };

    const { container } = render(
      <TurnNodeRenderer nodes={[node]} sessionId="session-1" />,
    );

    expect(container.textContent).toContain("已读取");
    expect(container.textContent).toContain("README.md");
    fireEvent.click(container.querySelector("button")!);
    expect(container.textContent).toContain("hello");
    expect(api.getToolOutputDetail).not.toHaveBeenCalled();
  });

  it("tool_step lazily loads large output details through the session endpoint", async () => {
    const node: ToolStepNode = {
      kind: "tool_step",
      node_id: "tool-large",
      turn_id: "t1",
      status: "completed",
      created_at_ms: 1000,
      updated_at_ms: 1200,
      tool_name: "grep",
      tool_category: "search",
      display_title: "Search matches",
      call_id: "tc-large",
      output_detail: {
        handle: "out_large",
        byte_length: 50_000,
        line_count: 1200,
        is_expandable: true,
        size_class: "large",
        summary: "1,200 lines",
        content_type: "search_results",
      },
    };

    const { container } = render(
      <TurnNodeRenderer nodes={[node]} sessionId="session-1" />,
    );

    fireEvent.click(container.querySelector("button")!);
    fireEvent.click(container.querySelectorAll("button")[1]);

    await waitFor(() => {
      expect(api.getToolOutputDetail).toHaveBeenCalledWith("session-1", "out_large");
    });
    await waitFor(() => {
      expect(container.textContent).toContain("line 1");
    });
  });

  it("tool_group expands steps in original order", () => {
    const first: ToolStepNode = {
      kind: "tool_step",
      node_id: "tool-1",
      turn_id: "t1",
      status: "completed",
      created_at_ms: 1000,
      updated_at_ms: 1100,
      tool_name: "read_file",
      tool_category: "file",
      display_title: "Read a.txt",
      call_id: "tc-1",
      target: { path: "a.txt" },
    };
    const second: ToolStepNode = {
      ...first,
      node_id: "tool-2",
      display_title: "Read b.txt",
      call_id: "tc-2",
      target: { path: "b.txt" },
    };
    const group: ToolGroupNode = {
      kind: "tool_group",
      node_id: "group-1",
      turn_id: "t1",
      status: "completed",
      created_at_ms: 1000,
      updated_at_ms: 1200,
      group_label: "Read files",
      step_count: 2,
      steps: [first, second],
      collapsed: true,
    };

    const { container } = render(
      <TurnNodeRenderer nodes={[group]} sessionId="session-1" />,
    );

    expect(container.textContent).toContain("Read files");
    expect(container.textContent).not.toContain("a.txt");
    fireEvent.click(container.querySelector("button")!);
    expect(container.textContent?.indexOf("a.txt")).toBeLessThan(
      container.textContent?.indexOf("b.txt") ?? -1,
    );
  });

  it("tool_step covers running status, progress, target command, and sub-second duration", () => {
    const now = Date.now();
    const node = toolStep({
      status: "running",
      tool_name: "shell_exec",
      tool_category: "shell",
      display_title: "Run tests",
      target: { command: "pnpm test" },
      started_at_ms: now - 1500,
      duration_ms: 250,
      progress: 1.4,
      progress_label: "finishing",
      args: '{"command":"pnpm test"}',
    });

    const { container } = render(<TurnNodeRenderer nodes={[node]} />);

    expect(container.textContent).toContain("正在运行测试");
    expect(container.textContent).toContain("pnpm test");
    expect(container.textContent).toContain("finishing");
    expect(container.textContent).toContain("1.5s");
    const progressFill = container.querySelector(".h-full.rounded-full") as HTMLElement | null;
    expect(progressFill?.style.width).toBe("100%");

    fireEvent.click(container.querySelector("button")!);
    expect(container.textContent).toContain('"command": "pnpm test"');
  });

  it("tool_step covers failed and cancelled error branches", () => {
    const failed = toolStep({
      node_id: "failed",
      status: "failed",
      display_title: "Run command",
      error_message: "Command failed",
    });
    const cancelled = toolStep({
      node_id: "cancelled",
      status: "cancelled",
      display_title: "Cancel command",
      error_message: "User cancelled",
    });

    const { container } = render(
      <TurnNodeRenderer nodes={[failed, cancelled]} />,
    );

    const buttons = container.querySelectorAll("button");
    fireEvent.click(buttons[0]);
    fireEvent.click(buttons[1]);
    expect(container.textContent).toContain("Command failed");
    expect(container.textContent).toContain("User cancelled");
  });

  it("tool_step does not expand when there are no args or output details", () => {
    const node = toolStep({
      output_preview: undefined,
      output_detail: undefined,
      args: undefined,
      error_message: undefined,
    });

    const { container } = render(<TurnNodeRenderer nodes={[node]} />);
    const button = container.querySelector("button")!;

    expect(button.getAttribute("aria-expanded")).toBeNull();
    fireEvent.click(button);
    expect(container.textContent).not.toContain("Params");
    expect(container.textContent).not.toContain("Output");
  });

  it("tool_step hides inline preview when small-output policy rejects it", () => {
    const node = toolStep({
      output_preview: {
        content: "binary data",
        byte_length: 10,
        line_count: 1,
        estimated_tokens: 2,
        is_binary: true,
        content_type: "text",
      },
    });

    const { container } = render(<TurnNodeRenderer nodes={[node]} />);
    fireEvent.click(container.querySelector("button")!);
    expect(container.textContent).not.toContain("binary data");
    expect(api.getToolOutputDetail).not.toHaveBeenCalled();
  });

  it("tool_step renders json and search result structured output branches", () => {
    const jsonNode = toolStep({
      node_id: "json-tool",
      display_title: "Read JSON",
      output_preview: smallPreview('{"a":1}', "json"),
    });
    const searchNode = toolStep({
      node_id: "search-tool",
      display_title: "Search files",
      tool_category: "search",
      output_preview: {
        ...smallPreview(
          Array.from({ length: 22 }, (_, i) => `match-${i}`).join("\n"),
          "search_results",
        ),
        line_count: 22,
      },
    });

    const { container } = render(
      <TurnNodeRenderer nodes={[jsonNode, searchNode]} />,
    );

    const buttons = container.querySelectorAll("button");
    fireEvent.click(buttons[0]);
    fireEvent.click(buttons[1]);
    expect(container.textContent).toContain('"a": 1');
    expect(container.textContent).toContain("match-0");
    expect(container.textContent).toContain("+2 more results");
  });

  it("tool_step truncates long default text output branch", () => {
    const longText = `${Array.from({ length: 205 }, (_, i) => `line-${i}`).join("\n")}`;
    const node = toolStep({
      output_preview: {
        content: longText,
        byte_length: 1000,
        line_count: 10,
        estimated_tokens: 20,
        is_binary: false,
        content_type: "text",
      },
    });

    const { container } = render(<TurnNodeRenderer nodes={[node]} />);
    fireEvent.click(container.querySelector("button")!);
    expect(container.textContent).toContain("line-0");
    expect(container.textContent).toContain("…");
  });

  it("tool_step covers large-output head, range, tail, and tail continuation branches", async () => {
    const detailMock = vi.mocked(api.getToolOutputDetail);
    detailMock
      .mockResolvedValueOnce({
        content: "head content",
        truncated: true,
        total_bytes: 100,
        total_lines: 20,
        continuation: { next_offset: 10 },
      })
      .mockResolvedValueOnce({
        content: "range content",
        truncated: true,
        total_bytes: 100,
        total_lines: 20,
      })
      .mockResolvedValueOnce({
        content: "tail content",
        truncated: true,
        total_bytes: 100,
        total_lines: 20,
      })
      .mockResolvedValueOnce({
        content: "more tail content",
        truncated: false,
        total_bytes: 100,
        total_lines: 20,
      });

    const node = toolStep({
      node_id: "large-paged",
      output_detail: {
        handle: "out_paged",
        byte_length: 100,
        line_count: 20,
        is_expandable: true,
        summary: "paged output",
        content_type: "command_output",
      },
    });

    const { container } = render(
      <TurnNodeRenderer nodes={[node]} sessionId="session-1" />,
    );

    fireEvent.click(container.querySelector("button")!);
    fireEvent.click(container.querySelectorAll("button")[1]);
    await waitFor(() => expect(container.textContent).toContain("head content"));

    fireEvent.click(container.querySelectorAll("button")[1]);
    await waitFor(() => expect(container.textContent).toContain("range content"));
    expect(detailMock).toHaveBeenCalledWith("session-1", "out_paged", {
      range_start: 10,
      range_end: 65546,
    });

    fireEvent.click(container.querySelectorAll("button")[3]);
    await waitFor(() => expect(container.textContent).toContain("tail content"));
    expect(detailMock).toHaveBeenCalledWith("session-1", "out_paged", {
      tail_lines: 100,
    });

    fireEvent.click(container.querySelectorAll("button")[1]);
    await waitFor(() => expect(container.textContent).toContain("more tail content"));
    expect(detailMock).toHaveBeenCalledWith("session-1", "out_paged", {
      tail_lines: 300,
    });
  });

  it("tool_step renders large-output detail error branch", async () => {
    vi.mocked(api.getToolOutputDetail).mockRejectedValueOnce(new Error("expired"));
    const node = toolStep({
      output_detail: {
        handle: "out_expired",
        byte_length: 100,
        line_count: 20,
        is_expandable: true,
      },
    });

    const { container } = render(
      <TurnNodeRenderer nodes={[node]} sessionId="session-1" />,
    );

    fireEvent.click(container.querySelector("button")!);
    fireEvent.click(container.querySelectorAll("button")[1]);
    await waitFor(() => expect(container.textContent).toContain("expired"));
  });

  it("tool_step does not render detail controls for large output without session scope", () => {
    const node = toolStep({
      output_detail: {
        handle: "out_large",
        byte_length: 100,
        line_count: 20,
        is_expandable: true,
      },
    });

    const { container } = render(<TurnNodeRenderer nodes={[node]} />);

    fireEvent.click(container.querySelector("button")!);
    expect(container.textContent).not.toContain("Output detail");
    expect(api.getToolOutputDetail).not.toHaveBeenCalled();
  });
});

// ============================================================================
// Streaming vs completed states
// ============================================================================

describe("Streaming vs completed rendering", () => {
  it("pending text node with isLive=true renders without crash", async () => {
    const node: AssistantTextNode = {
      kind: "assistant_text",
      node_id: "at-1",
      turn_id: "t1",
      status: "pending",
      created_at_ms: 1000,
      updated_at_ms: 1000,
      content: "Streaming\nwith\nnewlines",
    };

    const { container } = render(
      <TurnNodeRenderer nodes={[node]} isLive />,
    );
    // StreamingMarkdown shows ActiveLine for the last line (after last newline)
    await waitFor(() => {
      expect(container.textContent).toContain("newlines");
    }, { timeout: 5000 });
  });
});

// ============================================================================
// Replay hydration equivalence
// ============================================================================

describe("Replay hydration equivalence", () => {
  it("replay nodes match live reducer nodes for simple turn", () => {
    const events = simpleTextTurnFixture();
    const liveState = reduceTimelineEvents(events);
    const replayNodes = materializeNodes(events);

    // Same number of nodes
    expect(replayNodes.length).toBe(liveState.nodes.length);

    // Same kinds in same order
    expect(replayNodes.map((n) => n.kind)).toEqual(
      liveState.nodes.map((n) => n.kind),
    );
  });

  it("replay nodes match live reducer nodes for complex turn", () => {
    const events = complexTurnFixture();
    const liveState = reduceTimelineEvents(events);
    const replayNodes = materializeNodes(events);

    expect(replayNodes.map((n) => n.kind)).toEqual(
      liveState.nodes.map((n) => n.kind),
    );
  });

  it("replay nodes match live reducer nodes for tool loop turn", () => {
    const events = toolLoopTerminationFixture();
    const liveState = reduceTimelineEvents(events);
    const replayNodes = materializeNodes(events);

    expect(replayNodes.map((n) => n.kind)).toEqual(
      liveState.nodes.map((n) => n.kind),
    );

    // Both have failing turn_status
    const liveStatus = liveState.nodes.find(
      (n) => n.kind === "turn_status",
    ) as TurnStatusNode;
    const replayStatus = replayNodes.find(
      (n) => n.kind === "turn_status",
    ) as TurnStatusNode;
    expect(liveStatus.end_reason).toBe(replayStatus.end_reason);
    expect(liveStatus.status).toBe("failed");
    expect(replayStatus.status).toBe("failed");
  });

  it("both live and replay render without crashing", () => {
    const events = complexTurnFixture();
    const liveState = reduceTimelineEvents(events);
    const replayNodes = materializeNodes(events);

    const { container: liveContainer } = render(
      <TurnNodeRenderer nodes={liveState.nodes} />,
    );
    const { container: replayContainer } = render(
      <TurnNodeRenderer nodes={replayNodes} />,
    );

    // Both should produce non-empty output
    expect(liveContainer.innerHTML.length).toBeGreaterThan(0);
    expect(replayContainer.innerHTML.length).toBeGreaterThan(0);
  });
});

// ============================================================================
// High-frequency delta resilience
// ============================================================================

describe("High-frequency delta resilience", () => {
  it("reducer handles rapid text deltas without node explosion", () => {
    let state = emptyTimelineState("test");

    // Simulate 50 rapid text deltas
    for (let i = 0; i < 50; i++) {
      const event = makeTextDelta({
        seq: i + 1,
        turn_id: "t1",
        payload: {
          node_id: "at-1",
          delta: `word${i} `,
        },
      });
      state = reduceTimelineEvent(state, event);
    }

    // Should still be only 1 text node (coalesced)
    const textNodes = state.nodes.filter(
      (n) => n.kind === "assistant_text",
    );
    expect(textNodes.length).toBe(1);
  });
});

// ============================================================================
// Turn grouping (Section 10 — Codex/ChatGPT message blocks)
// ============================================================================

describe("Turn grouping (selectTurnGroups)", () => {
  it("partitions flat nodes into turn groups with user message and assistant nodes", () => {
    const events = simpleTextTurnFixture();
    const state = reduceTimelineEvents(events);

    const groups = selectTurnGroups(state);
    expect(groups.length).toBeGreaterThanOrEqual(1);

    const firstGroup = groups[0];
    expect(firstGroup.userMessageNode).not.toBeNull();
    expect(firstGroup.userMessageNode!.kind).toBe("user_message");
    expect(firstGroup.assistantNodes.length).toBeGreaterThanOrEqual(1);
    // Assistant nodes should NOT include user_message
    expect(firstGroup.assistantNodes.some((n) => n.kind === "user_message")).toBe(false);
  });

  it("groups multiple turns in order", () => {
    const events = [
      // First turn
      makeTurnStarted({ seq: 1, turn_id: "t1" }),
      makeUserMessageCreated({ seq: 2, turn_id: "t1", payload: { content: "First question" } }),
      makeTextDelta({ seq: 3, turn_id: "t1", payload: { node_id: "at-1", delta: "First answer" } }),
      makeTurnFinished({ seq: 4, turn_id: "t1", payload: { end_reason: "completed" } }),
      // Second turn
      makeTurnStarted({ seq: 5, turn_id: "t2" }),
      makeUserMessageCreated({ seq: 6, turn_id: "t2", payload: { content: "Second question" } }),
      makeTextDelta({ seq: 7, turn_id: "t2", payload: { node_id: "at-2", delta: "Second answer" } }),
      makeTurnFinished({ seq: 8, turn_id: "t2", payload: { end_reason: "completed" } }),
    ];
    const state = reduceTimelineEvents(events);
    const groups = selectTurnGroups(state);

    expect(groups.length).toBe(2);
    expect(groups[0].turnId).toBe("t1");
    expect(groups[0].userMessageNode?.content).toBe("First question");
    expect(groups[1].turnId).toBe("t2");
    expect(groups[1].userMessageNode?.content).toBe("Second question");
    expect(groups[1].assistantNodes.some((n) => n.kind === "assistant_text" && (n as AssistantTextNode).content.includes("Second answer"))).toBe(true);
  });

  it("preserves global order for interleaved turn continuations", () => {
    const events = [
      makeTurnStarted({ seq: 1, turn_id: "t1" }),
      makeUserMessageCreated({ seq: 2, turn_id: "t1", payload: { content: "First question" } }),
      makeTextDelta({ seq: 3, turn_id: "t1", payload: { node_id: "at-1a", delta: "First part." } }),
      makeTurnStarted({ seq: 4, turn_id: "t2" }),
      makeUserMessageCreated({ seq: 5, turn_id: "t2", payload: { content: "Second question" } }),
      makeTextDelta({ seq: 6, turn_id: "t2", payload: { node_id: "at-2", delta: "Second answer." } }),
      makeTextDelta({ seq: 7, turn_id: "t1", payload: { node_id: "at-1b", delta: "First continuation." } }),
    ];
    const state = reduceTimelineEvents(events);
    const groups = selectTurnGroups(state);

    expect(groups.map((g) => g.turnId)).toEqual(["t1", "t2", "t1"]);
    expect(groups[0].userMessageNode?.content).toBe("First question");
    expect(groups[1].userMessageNode?.content).toBe("Second question");
    expect(groups[2].userMessageNode).toBeNull();
    expect((groups[2].assistantNodes[0] as AssistantTextNode).content).toContain("First continuation.");
  });

  it("handles system-initiated turns (no user message)", () => {
    const state = emptyTimelineState("sys-test");
    // Add system notice without a user message
    let s = reduceTimelineEvent(state, makeTurnStarted({ seq: 1, turn_id: "sys-1" }));
    s = reduceTimelineEvent(s, { id: "evt-sys", session_id: "sys-test", turn_id: "sys-1", seq: 2, schema_version: 1, event_type: "system_notice", payload_json: { node_id: "sn-1", category: "compaction", level: "info", message: "Context compacted." } as unknown as Record<string, unknown>, created_at_ms: 1000 });
    s = reduceTimelineEvent(s, makeTurnFinished({ seq: 3, turn_id: "sys-1", payload: { end_reason: "completed" } }));

    const groups = selectTurnGroups(s);
    expect(groups.length).toBe(1);
    expect(groups[0].userMessageNode).toBeNull();
    expect(groups[0].assistantNodes.length).toBeGreaterThan(0);
  });
});

// ============================================================================
// TurnBlock rendering
// ============================================================================

describe("TurnBlock rendering", () => {
  it("renders user message followed by assistant response nodes", async () => {
    const events = simpleTextTurnFixture();
    const state = reduceTimelineEvents(events);
    const groups = selectTurnGroups(state);

    const { container } = render(
      <TurnBlock turnGroup={groups[0]} sessionId="session-1" />,
    );

    // User message content should appear
    expect(container.textContent).toContain("What is 2+2?");
    // Assistant response should appear
    await waitFor(() => {
      expect(container.textContent).toContain("2+2 = 4");
    }, { timeout: 5000 });
  });

  it("renders turn with only user message (no assistant response yet)", () => {
    const group: TurnGroup = {
      groupId: "pending-turn:0",
      turnId: "pending-turn",
      userMessageNode: {
        kind: "user_message",
        node_id: "um-1",
        turn_id: "pending-turn",
        status: "completed",
        created_at_ms: 1000,
        updated_at_ms: 1000,
        content: "Hello?",
      },
      assistantNodes: [],
    };

    const { container } = render(
      <TurnBlock turnGroup={group} />,
    );

    expect(container.textContent).toContain("Hello?");
    // No assistant response block should be rendered
    expect(container.querySelector(".assistant-response")).toBeNull();
  });

  it("renders user messages as a right-aligned Codex App-style bubble", () => {
    const group: TurnGroup = {
      groupId: "short-user:0",
      turnId: "short-user",
      userMessageNode: {
        kind: "user_message",
        node_id: "um-short",
        turn_id: "short-user",
        status: "completed",
        created_at_ms: 1000,
        updated_at_ms: 1000,
        content: "review 下代码",
      },
      assistantNodes: [],
    };

    const { container } = render(<TurnBlock turnGroup={group} />);
    const textEl = container.querySelector('[class*="whitespace-pre-wrap"]') as HTMLElement | null;

    expect(textEl?.textContent).toBe("review 下代码");
    expect(textEl?.style.wordBreak).toBe("normal");
    const userBubble = container.querySelector(".group\\/user-input") as HTMLElement | null;
    expect(userBubble).toBeTruthy();
    expect(userBubble?.style.justifyContent).toBe("flex-end");
  });
});

// ============================================================================
// Ordering within assistant response blocks (Task 10.6 / 10.7)
// ============================================================================

describe("Ordering within assistant response blocks", () => {
  it("preserves reasoning → tool → reasoning → text order", () => {
    const events = [
      makeTurnStarted({ seq: 1, turn_id: "t-order" }),
      makeUserMessageCreated({ seq: 2, turn_id: "t-order", payload: { content: "Do the thing" } }),
      makeReasoningDelta({ seq: 3, turn_id: "t-order", payload: { node_id: "r-1", delta: "Let me think..." } }),
      makeToolStarted({ seq: 4, turn_id: "t-order", payload: { call_id: "tc-1", tool_name: "grep", tool_category: "search", display_title: "Search files", args: "{}" } }),
      makeToolFinished({ seq: 5, turn_id: "t-order", payload: { call_id: "tc-1", tool_name: "grep", success: true, output_preview: { content: "found", byte_length: 5, line_count: 1, estimated_tokens: 1, is_binary: false, content_type: "text" } } }),
      makeReasoningDelta({ seq: 6, turn_id: "t-order", payload: { node_id: "r-2", delta: "Now I know..." } }),
      makeTextDelta({ seq: 7, turn_id: "t-order", payload: { node_id: "at-order", delta: "Here is the result." } }),
      makeTurnFinished({ seq: 8, turn_id: "t-order", payload: { end_reason: "completed" } }),
    ];
    const state = reduceTimelineEvents(events);
    const groups = selectTurnGroups(state);
    const assistantNodes = groups[0].assistantNodes;

    const kinds = assistantNodes.map((n) => n.kind);
    // Should have reasoning, tool_step, reasoning, assistant_text, turn_status in that order
    // (turn_finished always appends a turn_status node)
    const nonStatusKinds = kinds.filter((k) => k !== "turn_status");
    expect(nonStatusKinds).toEqual(["reasoning", "tool_step", "reasoning", "assistant_text"]);

    const { container } = render(
      <TurnBlock turnGroup={groups[0]} sessionId="session-1" />,
    );
    const response = container.querySelector(".assistant-response")!;
    const presentationKinds = Array.from(response.querySelectorAll("[data-timeline-node-kind], [data-presentation-kind]"))
      .map((el) => el.getAttribute("data-presentation-kind") ?? el.getAttribute("data-timeline-node-kind"))
      .filter((kind) => kind !== "turn_status");
    // Completed turns fold the whole process summary, keeping final text outside.
    const intervalKind = presentationKinds[0];
    expect(intervalKind).toBe("completed_turn_process");
    expect(presentationKinds[presentationKinds.length - 1]).toBe("assistant_text");
    expect(container.textContent).toContain("已处理");
    expect(container.textContent).toContain("Here is the result.");
    expect(container.textContent).not.toContain("已搜索 1 次");
    expect(container.textContent).not.toContain("Let me think");

    const processBtn = response.querySelector('[data-presentation-kind="completed_turn_process"] button');
    expect(processBtn).toBeTruthy();
    fireEvent.click(processBtn!);
    expect(container.textContent).toContain("已搜索 1 次");
  });

  it("preserves text → tool → text order", () => {
    const events = [
      makeTurnStarted({ seq: 1, turn_id: "t-ttt" }),
      makeUserMessageCreated({ seq: 2, turn_id: "t-ttt", payload: { content: "Check file" } }),
      makeTextDelta({ seq: 3, turn_id: "t-ttt", payload: { node_id: "at-1", delta: "Let me check the file." } }),
      makeToolStarted({ seq: 4, turn_id: "t-ttt", payload: { call_id: "tc-ttt", tool_name: "read_file", tool_category: "file", display_title: "Read file", args: "{}" } }),
      makeToolFinished({ seq: 5, turn_id: "t-ttt", payload: { call_id: "tc-ttt", tool_name: "read_file", success: true } }),
      makeTextDelta({ seq: 6, turn_id: "t-ttt", payload: { node_id: "at-2", delta: "The file contains..." } }),
      makeTurnFinished({ seq: 7, turn_id: "t-ttt", payload: { end_reason: "completed" } }),
    ];
    const state = reduceTimelineEvents(events);
    const groups = selectTurnGroups(state);
    const assistantNodes = groups[0].assistantNodes;

    const kinds = assistantNodes.map((n) => n.kind);
    // Text nodes should be separate (not merged across tool boundary)
    // (turn_finished always appends a turn_status node)
    const nonStatusKinds = kinds.filter((k) => k !== "turn_status");
    expect(nonStatusKinds).toEqual(["assistant_text", "tool_step", "assistant_text"]);
  });

  it("renders text → tool → text in correct DOM order", async () => {
    const events = [
      makeTurnStarted({ seq: 1, turn_id: "t-dom" }),
      makeUserMessageCreated({ seq: 2, turn_id: "t-dom", payload: { content: "Check" } }),
      makeTextDelta({ seq: 3, turn_id: "t-dom", payload: { node_id: "at-before", delta: "Before tool." } }),
      makeToolStarted({ seq: 4, turn_id: "t-dom", payload: { call_id: "tc-dom", tool_name: "ls", tool_category: "shell", display_title: "List files", args: "{}" } }),
      makeToolFinished({ seq: 5, turn_id: "t-dom", payload: { call_id: "tc-dom", tool_name: "ls", success: true } }),
      makeTextDelta({ seq: 6, turn_id: "t-dom", payload: { node_id: "at-after", delta: "After tool." } }),
      makeTurnFinished({ seq: 7, turn_id: "t-dom", payload: { end_reason: "completed" } }),
    ];
    const state = reduceTimelineEvents(events);

    const groups = selectTurnGroups(state);
    const { container } = render(
      <TurnBlock turnGroup={groups[0]} sessionId="session-1" />,
    );

    await waitFor(() => {
      expect(container.textContent).toContain("Before tool.");
      expect(container.textContent).toContain("After tool.");
    });

    const response = container.querySelector(".assistant-response")!;
    const domKinds = Array.from(response.querySelectorAll("[data-timeline-node-kind], [data-presentation-kind]"))
      .map((el) => el.getAttribute("data-presentation-kind") ?? el.getAttribute("data-timeline-node-kind"))
      .filter((kind) => kind !== "turn_status");
    expect(domKinds).toEqual(["assistant_text", "completed_turn_process", "assistant_text"]);

    fireEvent.click(response.querySelector('[data-presentation-kind="completed_turn_process"] button')!);
    const text = container.textContent ?? "";
    expect(text.indexOf("Before tool.")).toBeLessThan(text.indexOf("已运行 1 条命令"));
    expect(text.indexOf("已运行 1 条命令")).toBeLessThan(text.indexOf("After tool."));
    fireEvent.click(response.querySelector('[data-presentation-kind="process_interval"] button')!);
    expect(container.textContent).toContain("已运行命令");
  });
});

// ============================================================================
// Iteration boundary diagnostics (Task 10.5)
// ============================================================================

describe("Iteration boundary diagnostics", () => {
  it("hides iteration label in default UI", () => {
    const node: IterationBoundaryNode = {
      kind: "iteration_boundary",
      node_id: "ib-d",
      turn_id: "t1",
      status: "completed",
      created_at_ms: 1000,
      updated_at_ms: 1000,
      iteration: 5,
    };

    const { container } = render(
      <TurnNodeRenderer nodes={[node]} />,
    );

    const label = container.querySelector(".iteration-label") as HTMLElement | null;
    expect(label).toBeNull();
    expect(container.textContent).not.toContain("Iteration 5");
  });

  it("shows iteration label when data-diagnostics is enabled", () => {
    const node: IterationBoundaryNode = {
      kind: "iteration_boundary",
      node_id: "ib-v",
      turn_id: "t1",
      status: "completed",
      created_at_ms: 1000,
      updated_at_ms: 1000,
      iteration: 3,
    };

    const { container } = render(
      <div data-diagnostics="true">
        <TurnNodeRenderer nodes={[node]} showDiagnostics />
      </div>,
    );

    // With data-diagnostics, the iteration label should be visible
    expect(container.textContent).toContain("Iteration 3");
  });

  it("boundary remains as thin divider, not shown as a chat message", () => {
    const node: IterationBoundaryNode = {
      kind: "iteration_boundary",
      node_id: "ib-div",
      turn_id: "t1",
      status: "completed",
      created_at_ms: 1000,
      updated_at_ms: 1000,
      iteration: 2,
    };

    const { container } = render(
      <TurnNodeRenderer nodes={[node]} />,
    );

    const hiddenEl = container.querySelector('[aria-hidden="true"]');
    expect(hiddenEl).toBeNull();
    expect(container.innerHTML).toBe("");
  });
});

// ============================================================================
// Codex/ChatGPT layout visual structure
// ============================================================================

describe("Codex/ChatGPT layout visual structure", () => {
  it("assistant response block wraps tool steps and reasoning as activity rows", () => {
    const events = complexTurnFixture();
    const state = reduceTimelineEvents(events);
    const groups = selectTurnGroups(state);

    const { container } = render(
      <TurnBlock turnGroup={groups[0]} sessionId="session-1" />,
    );

    // Should have assistant-response container
    expect(container.querySelector(".assistant-response")).toBeTruthy();
    // Tool steps and reasoning should be inside the assistant response
    const response = container.querySelector(".assistant-response")!;
    expect(response.innerHTML.length).toBeGreaterThan(0);
    expect(response.querySelectorAll("[data-activity-row]").length).toBeGreaterThan(0);
  });

  it("tool loop termination status appears within assistant response block", () => {
    const events = toolLoopTerminationFixture();
    const state = reduceTimelineEvents(events);
    const groups = selectTurnGroups(state);

    const { container } = render(
      <TurnBlock turnGroup={groups[0]} sessionId="session-1" />,
    );

    // Turn status (abnormal) should be inside the assistant response
    const response = container.querySelector(".assistant-response");
    expect(response).toBeTruthy();
    expect(container.textContent).toContain("tool loop");
  });

  it("covers running reasoning, running tool, completed tool, resumed text, and abnormal status inside one assistant response", async () => {
    const events = [
      makeTurnStarted({ seq: 1, turn_id: "t-layout" }),
      makeUserMessageCreated({ seq: 2, turn_id: "t-layout", payload: { content: "Investigate" } }),
      makeReasoningDelta({ seq: 3, turn_id: "t-layout", payload: { node_id: "r-live", delta: "Checking context." } }),
      makeToolStarted({ seq: 4, turn_id: "t-layout", payload: { call_id: "tc-running", tool_name: "grep", tool_category: "search", display_title: "Search code", args: "{}" } }),
      makeToolStarted({ seq: 5, turn_id: "t-layout", payload: { call_id: "tc-done", tool_name: "read_file", tool_category: "file", display_title: "Read file", args: "{}" } }),
      makeToolFinished({
        seq: 6,
        turn_id: "t-layout",
        payload: {
          call_id: "tc-done",
          tool_name: "read_file",
          success: true,
          output_preview: smallPreview("file body") as unknown as Record<string, unknown>,
        },
      }),
      makeTextDelta({ seq: 7, turn_id: "t-layout", payload: { node_id: "at-layout", delta: "The answer resumes here." } }),
      makeTurnFinished({ seq: 8, turn_id: "t-layout", payload: { end_reason: "tool_loop" } }),
    ];
    const state = reduceTimelineEvents(events);
    const groups = selectTurnGroups(state);

    const { container } = render(
      <TurnBlock turnGroup={groups[0]} isLive sessionId="session-1" />,
    );

    await waitFor(() => {
      const response = container.querySelector(".assistant-response")!;
      const domKinds = Array.from(response.querySelectorAll("[data-timeline-node-kind], [data-presentation-kind]"))
        .map((el) => el.getAttribute("data-presentation-kind") ?? el.getAttribute("data-timeline-node-kind"));
      // New presentation: running tool separated from completed intervals
      expect(domKinds.filter(k => k === "process_interval").length).toBeGreaterThanOrEqual(1);
      expect(domKinds).toContain("assistant_text");
      expect(domKinds).toContain("turn_status");
      expect(response.querySelectorAll("[data-activity-row]").length).toBeGreaterThanOrEqual(2);
      expect(container.textContent).toContain("The answer resumes here.");
      expect(response.querySelector('[data-timeline-node-kind="turn_status"]')).toBeTruthy();
    });

    const response = container.querySelector(".assistant-response")!;
    const intervalBtn = response.querySelector('[data-presentation-kind="process_interval"] button');
    if (intervalBtn) {
      fireEvent.click(intervalBtn);
      const expandedKinds = Array.from(response.querySelectorAll("[data-completed-process-transcript] [data-timeline-node-kind], [data-completed-process-transcript] [data-presentation-kind]"))
        .map((el) => el.getAttribute("data-presentation-kind") ?? el.getAttribute("data-timeline-node-kind"));
      // Expanded process interval shows individual nodes
      expect(expandedKinds.length).toBeGreaterThanOrEqual(1);
    }
  });

  it("keeps live process status active after tools finish while waiting for answer text", () => {
    const events = [
      makeTurnStarted({ seq: 1, turn_id: "t-live-thinking" }),
      makeUserMessageCreated({ seq: 2, turn_id: "t-live-thinking", payload: { content: "Review code" } }),
      makeToolStarted({
        seq: 3,
        turn_id: "t-live-thinking",
        payload: { call_id: "tc-done", tool_name: "read_file", tool_category: "file", display_title: "Read file", args: "{}" },
      }),
      makeToolFinished({ seq: 4, turn_id: "t-live-thinking", payload: { call_id: "tc-done", tool_name: "read_file", success: true } }),
    ];
    const state = reduceTimelineEvents(events);
    const groups = selectTurnGroups(state);

    const { container } = render(
      <TurnBlock turnGroup={groups[0]} isLive sessionId="session-1" />,
    );

    expect(container.textContent).toContain("正在阅读相关实现");
    expect(container.textContent).toContain("已读取 1 个文件");
    expect(container.textContent).not.toContain("已运行 1 条命令Read file");
  });

  it("shows the active running tool and completed count in live process status", () => {
    const events = [
      makeTurnStarted({ seq: 1, turn_id: "t-live-running" }),
      makeUserMessageCreated({ seq: 2, turn_id: "t-live-running", payload: { content: "Review code" } }),
      makeToolStarted({
        seq: 3,
        turn_id: "t-live-running",
        payload: { call_id: "tc-done", tool_name: "read_file", tool_category: "file", display_title: "Read file", args: "{}" },
      }),
      makeToolFinished({ seq: 4, turn_id: "t-live-running", payload: { call_id: "tc-done", tool_name: "read_file", success: true } }),
      makeToolStarted({
        seq: 5,
        turn_id: "t-live-running",
        payload: {
          call_id: "tc-running",
          tool_name: "shell_exec",
          tool_category: "shell",
          display_title: "Run command",
          args: JSON.stringify({ command: "pnpm test -- src/components/message-stream/__tests__/turn-node-renderer.test.tsx" }),
        },
      }),
    ];
    const state = reduceTimelineEvents(events);
    const groups = selectTurnGroups(state);

    const { container } = render(
      <TurnBlock turnGroup={groups[0]} isLive sessionId="session-1" />,
    );

    expect(container.textContent).toContain("正在运行测试");
    expect(container.textContent).toContain("pnpm test");
    expect(container.textContent).toContain("已读取 1 个文件");
  });

  it("summarizes sub-agent spawn/get tools without exposing raw function names by default", () => {
    const events = [
      makeTurnStarted({ seq: 1, turn_id: "t-subagents" }),
      makeUserMessageCreated({ seq: 2, turn_id: "t-subagents", payload: { content: "Review changes" } }),
      makeToolStarted({
        seq: 3,
        turn_id: "t-subagents",
        payload: {
          call_id: "spawn-1",
          tool_name: "spawn_subagent",
          tool_category: "sub_agent",
          display_title: "Sub-agent",
          args: "{}",
        },
      }),
      makeToolFinished({ seq: 4, turn_id: "t-subagents", payload: { call_id: "spawn-1", tool_name: "spawn_subagent", success: true } }),
      makeToolStarted({
        seq: 5,
        turn_id: "t-subagents",
        payload: {
          call_id: "get-1",
          tool_name: "subagent_get",
          tool_category: "sub_agent",
          display_title: "subagent_get",
          args: "{}",
        },
      }),
      makeToolFinished({ seq: 6, turn_id: "t-subagents", payload: { call_id: "get-1", tool_name: "subagent_get", success: true } }),
    ];
    const state = reduceTimelineEvents(events);
    const groups = selectTurnGroups(state);

    const { container } = render(
      <TurnBlock turnGroup={groups[0]} sessionId="session-1" />,
    );

    const response = container.querySelector(".assistant-response")!;
    // Completed tools are inside process_intervals
    const interval = response.querySelector('[data-presentation-kind="process_interval"]');
    expect(interval).toBeTruthy();
    // Expand to see tool details
    const btn = interval?.querySelector("button");
    if (btn) fireEvent.click(btn);
    expect(container.textContent).toContain("已调用子代理");
  });

  it("keeps diff inspection and sub-agent review as separate activity groups", () => {
    const events = [
      makeTurnStarted({ seq: 1, turn_id: "t-mixed-tools" }),
      makeUserMessageCreated({ seq: 2, turn_id: "t-mixed-tools", payload: { content: "Review code" } }),
      makeToolStarted({
        seq: 3,
        turn_id: "t-mixed-tools",
        payload: {
          call_id: "diff-1",
          tool_name: "shell_exec",
          tool_category: "shell",
          display_title: "Run git diff --stat",
        },
      }),
      makeToolFinished({ seq: 4, turn_id: "t-mixed-tools", payload: { call_id: "diff-1", tool_name: "shell_exec", success: true } }),
      makeToolStarted({
        seq: 5,
        turn_id: "t-mixed-tools",
        payload: {
          call_id: "spawn-1",
          tool_name: "spawn_subagent",
          tool_category: "sub_agent",
          display_title: "Sub-agent",
          args: "{}",
        },
      }),
      makeToolFinished({ seq: 6, turn_id: "t-mixed-tools", payload: { call_id: "spawn-1", tool_name: "spawn_subagent", success: true } }),
    ];
    const state = reduceTimelineEvents(events);
    const groups = selectTurnGroups(state);

    const { container } = render(
      <TurnBlock turnGroup={groups[0]} sessionId="session-1" />,
    );

    // Completed tools are folded into process intervals
    const intervalEl = container.querySelector('[data-presentation-kind="process_interval"]');
    expect(intervalEl).toBeTruthy();
    // Expand to see tool details
    const btn = intervalEl?.querySelector("button");
    if (btn) fireEvent.click(btn);
    expect(container.textContent).toContain("已运行命令");
  });
});
