import { describe, expect, it } from "vitest";
import {
  derivePresentationMode,
  selectAssistantTurnPresentation,
  buildPresentationItems,
} from "../timeline";
import type { AssistantTextNode, ReasoningNode, ToolStepNode, TurnDisplayNode, TurnStatusNode } from "../timeline";

const base = {
  turn_id: "t1",
  created_at_ms: 1000,
  updated_at_ms: 1000,
} as const;

function text(node_id: string, content: string, text_role?: "activity" | "final"): AssistantTextNode {
  return {
    kind: "assistant_text",
    node_id,
    status: "completed",
    content,
    text_role,
    ...base,
  };
}

function reasoning(node_id: string, status: ReasoningNode["status"] = "completed"): ReasoningNode {
  return {
    kind: "reasoning",
    node_id,
    status,
    visibility: "public",
    content: `reasoning ${node_id}`,
    ...base,
  };
}

function tool(node_id: string, status: ToolStepNode["status"] = "completed"): ToolStepNode {
  return {
    kind: "tool_step",
    node_id,
    status,
    tool_name: "shell_exec",
    tool_category: "shell",
    display_title: "Run git diff --stat",
    call_id: node_id,
    ...base,
  };
}

function status(end_reason: string, nodeStatus: TurnStatusNode["status"] = "completed"): TurnStatusNode {
  return {
    kind: "turn_status",
    node_id: `status-${end_reason}`,
    status: nodeStatus,
    end_reason,
    elapsed_ms: 28_000,
    ...base,
  };
}

describe("timeline presentation selector", () => {
  it("keeps running turns active when process nodes are pending or running", () => {
    const nodes: TurnDisplayNode[] = [
      reasoning("r-live", "pending"),
      tool("tool-live", "running"),
      text("answer", "Still working"),
    ];

    const presentation = selectAssistantTurnPresentation(nodes);

    expect(presentation.mode).toBe("active");
    expect(derivePresentationMode(nodes)).toBe("active");
    // Active mode: all nodes appear individually
    expect(presentation.items.length).toBeGreaterThan(0);
  });

  it("folds completed process nodes into intervals while leaving answer text visible", () => {
    const nodes: TurnDisplayNode[] = [
      reasoning("r-1"),
      tool("tool-1"),
      reasoning("r-2"),
      text("answer", "Final answer", "final"),
      status("completed"),
    ];

    const items = buildPresentationItems(nodes);

    // Runtime narration + completed batch + final text. Reasoning is not exposed.
    const kinds = items.map((item) => item.kind);
    expect(kinds).toEqual(["narration", "completed_batch", "visible"]);

    const narration = items[0];
    if (narration.kind !== "narration") {
      throw new Error("expected narration");
    }
    expect(narration.narration.source).toBe("runtime");

    const interval = items[1];
    if (interval.kind !== "completed_batch") {
      throw new Error("expected completed_batch");
    }
    expect(interval.interval.nodes.map((node) => node.node_id)).toEqual(["tool-1"]);
    expect(interval.interval.durationMs).toBeGreaterThanOrEqual(0);

    const visible = items[2];
    if (visible.kind !== "visible") {
      throw new Error("expected visible item");
    }
    expect(visible.node.kind).toBe("assistant_text");
  });

  it("keeps abnormal terminal status as attention (not in folded process)", () => {
    const nodes: TurnDisplayNode[] = [
      reasoning("r-1"),
      tool("tool-1"),
      text("partial", "Partial answer"),
      status("tool_loop", "failed"),
    ];

    const items = buildPresentationItems(nodes);

    // Should have: narration + completed_batch + visible (partial text) + error (turn_status)
    const kinds = items.map((item) => item.kind);
    expect(kinds).toContain("completed_batch");
    expect(kinds).toContain("visible");
    expect(kinds).toContain("error");
  });

  it("skips non-public reasoning", () => {
    const privateR: ReasoningNode = {
      kind: "reasoning",
      node_id: "r-private",
      status: "completed",
      visibility: "private",
      content: "secret CoT",
      turn_id: "t1",
      created_at_ms: 1000,
      updated_at_ms: 1000,
    };

    const items = buildPresentationItems([privateR]);
    // Private reasoning should be completely absent
    expect(items).toHaveLength(0);
  });

  it("failed tools are attention items", () => {
    const failedTool: ToolStepNode = {
      kind: "tool_step",
      node_id: "tool-failed",
      status: "failed",
      tool_name: "shell_exec",
      tool_category: "shell",
      display_title: "Run tests",
      call_id: "tool-failed",
      turn_id: "t1",
      created_at_ms: 1000,
      updated_at_ms: 1000,
    };

    const items = buildPresentationItems([failedTool]);
    expect(items).toHaveLength(2);
    expect(items[0].kind).toBe("narration");
    expect(items[1].kind).toBe("failed_tool");
  });
});
