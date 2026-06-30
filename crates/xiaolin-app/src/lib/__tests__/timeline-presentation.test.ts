import { describe, expect, it } from "vitest";
import {
  derivePresentationMode,
  selectAssistantTurnPresentation,
} from "../timeline";
import type { AssistantTextNode, ReasoningNode, ToolStepNode, TurnDisplayNode, TurnStatusNode } from "../timeline";

const base = {
  turn_id: "t1",
  created_at_ms: 1000,
  updated_at_ms: 1000,
} as const;

function text(node_id: string, content: string): AssistantTextNode {
  return {
    kind: "assistant_text",
    node_id,
    status: "completed",
    content,
    ...base,
  };
}

function reasoning(node_id: string, status: ReasoningNode["status"] = "completed"): ReasoningNode {
  return {
    kind: "reasoning",
    node_id,
    status,
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
    expect(presentation.items.map((item) => item.kind)).toEqual(["visible", "visible", "visible"]);
  });

  it("folds completed process nodes into one summary while leaving answer text visible", () => {
    const nodes: TurnDisplayNode[] = [
      reasoning("r-1"),
      tool("tool-1"),
      reasoning("r-2"),
      text("answer", "Final answer"),
      status("completed"),
    ];

    const presentation = selectAssistantTurnPresentation(nodes);

    expect(presentation.mode).toBe("completed");
    expect(presentation.items.map((item) => item.kind)).toEqual(["process_summary", "visible"]);
    const summary = presentation.items[0];
    if (summary.kind !== "process_summary") {
      throw new Error("expected process summary");
    }
    expect(summary.nodes.map((node) => node.node_id)).toEqual(["r-1", "tool-1", "r-2"]);
    expect(summary.elapsedMs).toBe(28_000);
  });

  it("keeps abnormal terminal status visible outside folded process", () => {
    const nodes: TurnDisplayNode[] = [
      reasoning("r-1"),
      tool("tool-1"),
      text("partial", "Partial answer"),
      status("tool_loop", "failed"),
    ];

    const presentation = selectAssistantTurnPresentation(nodes);

    expect(presentation.mode).toBe("abnormal");
    expect(presentation.items.map((item) => item.kind)).toEqual(["process_summary", "visible", "visible"]);
    const last = presentation.items[2];
    if (last.kind !== "visible") {
      throw new Error("expected visible terminal status");
    }
    expect(last.node.kind).toBe("turn_status");
  });
});
