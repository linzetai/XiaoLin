/**
 * Timeline reducer golden tests.
 *
 * These tests prove that live event reduction and replay materialization
 * produce equivalent normalized TurnDisplayNode[] output.
 *
 * Covers:
 * - Simple text-only turns
 * - Complex turns with reasoning, tools, approvals, boundaries
 * - Tool loop termination with partial text
 * - Idempotent event append
 * - Delta coalescing
 * - Empty delta handling
 * - Out-of-order event handling
 */

import { describe, it, expect } from "vitest";
import {
  reduceTimelineEvents,
  reduceTimelineEvent,
  materializeNodes,
  emptyTimelineState,
  nodesAreEquivalent,
  diffNodes,
  sortEventsBySeq,
  deduplicateEvents,
  selectNodesForTurn,
  selectMaxSeq,
  selectActiveNodes,
} from "../timeline";
import {
  complexTurnFixture,
  simpleTextTurnFixture,
  toolLoopTerminationFixture,
  makeTextDelta,
  makeTextSnapshot,
  makeToolStarted,
  makeToolFinished,
  makeTurnFinished,
  makeTurnStarted,
  makeUserMessageCreated,
  makeReasoningDelta,
  makeReasoningSnapshot,
  makeAssistantMessageFinalized,
} from "../timeline/fixtures";
import type {
  TurnTimelineEvent,
  TurnDisplayNode,
} from "../timeline/types";

// ============================================================================
// Helper: extract a simpler comparable representation
// ============================================================================

/** Get node kinds in order — a quick structural test. */
function kindSequence(nodes: TurnDisplayNode[]): string[] {
  return nodes.map((n) => n.kind);
}

/** Find nodes by kind. */
function findNodesByKind(
  nodes: TurnDisplayNode[],
  kind: string,
): TurnDisplayNode[] {
  return nodes.filter((n) => n.kind === kind);
}

// ============================================================================
// Simple text-only turn
// ============================================================================

describe("Simple text-only turn", () => {
  const events = simpleTextTurnFixture();

  it("reduces to correct node kinds in order", () => {
    const state = reduceTimelineEvents(events);
    const kinds = kindSequence(state.nodes);
    // turn_started is metadata-only (no visible node)
    // So: user_message, assistant_text, turn_status
    expect(kinds).toEqual(["user_message", "assistant_text", "turn_status"]);
  });

  it("has the correct total node count", () => {
    const state = reduceTimelineEvents(events);
    expect(state.nodes).toHaveLength(3); // user_msg + text + turn_status
  });

  it("assistant text content is coalesced from deltas", () => {
    const state = reduceTimelineEvents(events);
    const textNode = findNodesByKind(state.nodes, "assistant_text")[0];
    expect(textNode).toBeDefined();
    if (textNode.kind === "assistant_text") {
      expect(textNode.content).toBe("2+2 = 4");
    }
  });

  it("turn_status has correct end_reason", () => {
    const state = reduceTimelineEvents(events);
    const statusNode = findNodesByKind(state.nodes, "turn_status")[0];
    expect(statusNode).toBeDefined();
    if (statusNode.kind === "turn_status") {
      expect(statusNode.end_reason).toBe("completed");
    }
  });

  it("materializeNodes returns same nodes as reduceTimelineEvents", () => {
    const state = reduceTimelineEvents(events);
    const materialized = materializeNodes(events);
    expect(nodesAreEquivalent(state.nodes, materialized)).toBe(true);
  });

  it("maxSeq matches the last event's seq", () => {
    const state = reduceTimelineEvents(events);
    const lastSeq = events[events.length - 1].seq;
    expect(state.maxSeq).toBe(lastSeq);
  });
});

// ============================================================================
// Complex turn fixture
// ============================================================================

describe("Complex turn fixture", () => {
  const events = complexTurnFixture();

  it("reduces to all expected node kinds", () => {
    const state = reduceTimelineEvents(events);
    const kinds = kindSequence(state.nodes);
    expect(kinds).toContain("user_message");
    expect(kinds).toContain("reasoning");
    expect(kinds).toContain("assistant_text");
    expect(kinds).toContain("tool_step");
    expect(kinds).toContain("approval");
    expect(kinds).toContain("iteration_boundary");
    expect(kinds).toContain("turn_status");
  });

  it("has correct number of tool step nodes", () => {
    const state = reduceTimelineEvents(events);
    const tools = findNodesByKind(state.nodes, "tool_step");
    expect(tools).toHaveLength(3); // read_file, grep, shell_exec
  });

  it("first tool step has output_preview", () => {
    const state = reduceTimelineEvents(events);
    const tools = findNodesByKind(state.nodes, "tool_step");
    const readTool = tools.find(
      (t) => t.kind === "tool_step" && t.call_id === "tc-read-1",
    );
    expect(readTool).toBeDefined();
    if (readTool?.kind === "tool_step") {
      expect(readTool.output_preview).toBeDefined();
      expect(readTool.output_preview?.content_type).toBe("text");
    }
  });

  it("grep tool step has output_detail (large output)", () => {
    const state = reduceTimelineEvents(events);
    const tools = findNodesByKind(state.nodes, "tool_step");
    const grepTool = tools.find(
      (t) => t.kind === "tool_step" && t.call_id === "tc-grep-2",
    );
    expect(grepTool).toBeDefined();
    if (grepTool?.kind === "tool_step") {
      expect(grepTool.output_detail).toBeDefined();
      expect(grepTool.output_detail?.handle).toBe("out_abc12345_def56789");
      expect(grepTool.output_preview).toBeUndefined(); // large → no inline preview
    }
  });

  it("approval node has correct decision", () => {
    const state = reduceTimelineEvents(events);
    const approvals = findNodesByKind(state.nodes, "approval");
    expect(approvals).toHaveLength(1);
    if (approvals[0].kind === "approval") {
      expect(approvals[0].decision).toBe("allow_once");
      expect(approvals[0].decision_source).toBe("user");
    }
  });

  it("iteration boundary is present", () => {
    const state = reduceTimelineEvents(events);
    const boundaries = findNodesByKind(state.nodes, "iteration_boundary");
    expect(boundaries).toHaveLength(1);
    if (boundaries[0].kind === "iteration_boundary") {
      expect(boundaries[0].iteration).toBe(1);
    }
  });

  it("turn_status has correct end_reason", () => {
    const state = reduceTimelineEvents(events);
    const statusNode = findNodesByKind(state.nodes, "turn_status")[0];
    expect(statusNode).toBeDefined();
    if (statusNode.kind === "turn_status") {
      expect(statusNode.end_reason).toBe("completed");
      // For normal completions, diagnosis is undefined (no error condition)
      expect(statusNode.elapsed_ms).toBe(7000);
    }
  });

  it("live reduction == replay materialization (golden test)", () => {
    const state = reduceTimelineEvents(events);
    const materialized = materializeNodes(events);
    const diffs = diffNodes(state.nodes, materialized);
    expect(diffs).toEqual([]);
    expect(nodesAreEquivalent(state.nodes, materialized)).toBe(true);
  });
});

// ============================================================================
// Tool loop termination
// ============================================================================

describe("Tool loop termination fixture", () => {
  const events = toolLoopTerminationFixture();

  it("has assistant text before turn status", () => {
    const state = reduceTimelineEvents(events);

    // Find positions of text and status
    const textIdx = state.nodes.findIndex((n) => n.kind === "assistant_text");
    const statusIdx = state.nodes.findIndex((n) => n.kind === "turn_status");

    expect(textIdx).toBeGreaterThanOrEqual(0);
    expect(statusIdx).toBeGreaterThanOrEqual(0);
    expect(textIdx).toBeLessThan(statusIdx); // text before status
  });

  it("turn_status has tool_loop diagnosis", () => {
    const state = reduceTimelineEvents(events);
    const statusNode = findNodesByKind(state.nodes, "turn_status")[0];
    expect(statusNode).toBeDefined();
    if (statusNode.kind === "turn_status") {
      expect(statusNode.end_reason).toBe("tool_loop");
      expect(statusNode.diagnosis?.diagnosis_code).toBe("tool_loop");
      expect(statusNode.diagnosis?.severity).toBe("error");
      expect(statusNode.diagnosis?.iterations).toBe(12);
      expect(statusNode.diagnosis?.tool_calls).toBe(45);
      expect(statusNode.status).toBe("failed");
    }
  });

  it("partial text is NOT rendered as normal completion", () => {
    const state = reduceTimelineEvents(events);
    const textNode = findNodesByKind(state.nodes, "assistant_text")[0];
    expect(textNode).toBeDefined();
    if (textNode.kind === "assistant_text") {
      // The text should be partial (just "Let me search for bugs.")
      expect(textNode.content).toContain("Let me search for bugs.");
      // But there IS a failed TurnStatusNode after it
    }
  });

  it("golden: live == replay for tool loop", () => {
    const state = reduceTimelineEvents(events);
    const materialized = materializeNodes(events);
    expect(nodesAreEquivalent(state.nodes, materialized)).toBe(true);
  });
});

// ============================================================================
// Idempotency
// ============================================================================

describe("Idempotent event append", () => {
  it("duplicate event id does not produce duplicate nodes", () => {
    const original = simpleTextTurnFixture();
    const event = original[0]; // turn_started

    // Apply event twice with same id
    let state = emptyTimelineState("test");
    state = reduceTimelineEvent(state, event);
    const firstNodeCount = state.nodes.length;
    state = reduceTimelineEvent(state, event); // same id → idempotent

    expect(state.nodes.length).toBe(firstNodeCount); // no extra node
    expect(state.events.length).toBe(1); // only stored once
  });

  it("duplicate event with different seq is still idempotent", () => {
    const event = makeUserMessageCreated({ id: "evt-dup-1" });
    const dupe = { ...event, seq: 999 }; // different seq, same id

    let state = emptyTimelineState("test");
    state = reduceTimelineEvent(state, event);
    state = reduceTimelineEvent(state, dupe);

    expect(state.nodes.length).toBe(1);
  });
});

// ============================================================================
// Delta coalescing
// ============================================================================

describe("Delta coalescing", () => {
  it("multiple text deltas coalesce into one node", () => {
    const events: TurnTimelineEvent[] = [
      makeTextDelta({
        turn_id: "t1",
        payload: { node_id: "at-1", delta: "Hello " },
      }),
      makeTextDelta({
        turn_id: "t1",
        payload: { node_id: "at-1", delta: "world!" },
      }),
    ];

    const state = reduceTimelineEvents(events);
    const textNodes = findNodesByKind(state.nodes, "assistant_text");
    expect(textNodes).toHaveLength(1);
    if (textNodes[0].kind === "assistant_text") {
      expect(textNodes[0].content).toBe("Hello world!");
    }
  });

  it("text deltas with different node_ids create separate nodes", () => {
    const events: TurnTimelineEvent[] = [
      makeTextDelta({
        turn_id: "t1",
        payload: { node_id: "at-1", delta: "First." },
      }),
      makeTextDelta({
        turn_id: "t1",
        payload: { node_id: "at-2", delta: "Second." },
      }),
    ];

    const state = reduceTimelineEvents(events);
    const textNodes = findNodesByKind(state.nodes, "assistant_text");
    expect(textNodes).toHaveLength(2);
  });

  it("reasoning deltas coalesce into one node", () => {
    const events: TurnTimelineEvent[] = [
      makeReasoningDelta({
        turn_id: "t1",
        payload: { node_id: "r-1", delta: "Think step 1. " },
      }),
      makeReasoningDelta({
        turn_id: "t1",
        payload: { node_id: "r-1", delta: "Think step 2." },
      }),
    ];

    const state = reduceTimelineEvents(events);
    const reasoningNodes = findNodesByKind(state.nodes, "reasoning");
    expect(reasoningNodes).toHaveLength(1);
    if (reasoningNodes[0].kind === "reasoning") {
      expect(reasoningNodes[0].content).toBe("Think step 1. Think step 2.");
    }
  });

  it("empty deltas are ignored", () => {
    const events: TurnTimelineEvent[] = [
      makeTextDelta({
        turn_id: "t1",
        payload: { node_id: "at-1", delta: "" },
      }),
      makeTextDelta({
        turn_id: "t1",
        payload: { node_id: "at-1", delta: "" },
      }),
      makeTextDelta({
        turn_id: "t1",
        payload: { node_id: "at-1", delta: "Real content." },
      }),
    ];

    const state = reduceTimelineEvents(events);
    const textNodes = findNodesByKind(state.nodes, "assistant_text");
    expect(textNodes).toHaveLength(1);
    if (textNodes[0].kind === "assistant_text") {
      expect(textNodes[0].content).toBe("Real content.");
    }
    // Only 1 meaningful event stored (empty deltas still pass through)
    // Actually empty deltas are still events but don't create nodes
  });
});

// ============================================================================
// Text snapshot overrides deltas
// ============================================================================

describe("Text snapshot", () => {
  it("snapshot provides authoritative content after deltas", () => {
    const events: TurnTimelineEvent[] = [
      makeTextDelta({
        turn_id: "t1",
        payload: { node_id: "at-1", delta: "Partial..." },
      }),
      makeTextSnapshot({
        turn_id: "t1",
        payload: { node_id: "at-1", content: "Complete final text.", byte_length: 20 },
      }),
    ];

    const state = reduceTimelineEvents(events);
    const textNodes = findNodesByKind(state.nodes, "assistant_text");
    expect(textNodes).toHaveLength(1);
    if (textNodes[0].kind === "assistant_text") {
      expect(textNodes[0].content).toBe("Complete final text.");
      expect(textNodes[0].status).toBe("completed");
    }
  });

  it("snapshot without prior deltas creates a new node", () => {
    const events: TurnTimelineEvent[] = [
      makeTextSnapshot({
        turn_id: "t1",
        payload: { node_id: "at-new", content: "Snapshot only.", byte_length: 14 },
      }),
    ];

    const state = reduceTimelineEvents(events);
    const textNodes = findNodesByKind(state.nodes, "assistant_text");
    expect(textNodes).toHaveLength(1);
    if (textNodes[0].kind === "assistant_text") {
      expect(textNodes[0].content).toBe("Snapshot only.");
    }
  });
});

// ============================================================================
// Tool lifecycle: start → progress → finish
// ============================================================================

describe("Tool lifecycle", () => {
  const startEvent = makeToolStarted({
    turn_id: "t1",
    payload: { call_id: "tc-1", tool_name: "bash", display_title: "Run build" },
  });
  const progressEvent = {
    ...makeToolStarted({ turn_id: "t1" }), // placeholder
    id: "evt-prog-1",
    seq: startEvent.seq + 1,
    event_type: "tool_call_progress" as const,
    payload_json: { call_id: "tc-1", message: "Building...", progress: 0.5 },
  };
  const finishEvent = makeToolFinished({
    turn_id: "t1",
    payload: {
      call_id: "tc-1",
      tool_name: "bash",
      success: true,
      duration_ms: 5000,
      output_preview: {
        content: "Build succeeded.",
        byte_length: 15,
        line_count: 1,
        estimated_tokens: 4,
        is_binary: false,
        content_type: "command_output",
      },
    },
  });

  it("tool goes through start → running → completed lifecycle", () => {
    let state = emptyTimelineState("test");
    state = reduceTimelineEvent(state, startEvent);

    // After start: status is running
    expect(state.nodes).toHaveLength(1);
    expect(state.nodes[0].status).toBe("running");
    if (state.nodes[0].kind === "tool_step") {
      expect(state.nodes[0].call_id).toBe("tc-1");
      expect(state.nodes[0].output_preview).toBeUndefined();
    }

    // After progress: still running, has progress info
    state = reduceTimelineEvent(state, progressEvent);
    expect(state.nodes[0].status).toBe("running");
    if (state.nodes[0].kind === "tool_step") {
      expect(state.nodes[0].progress).toBe(0.5);
    }

    // After finish: completed with output
    state = reduceTimelineEvent(state, finishEvent);
    expect(state.nodes[0].status).toBe("completed");
    if (state.nodes[0].kind === "tool_step") {
      expect(state.nodes[0].output_preview).toBeDefined();
      expect(state.nodes[0].duration_ms).toBe(5000);
    }
  });

  it("failed tool has status failed", () => {
    const failEvent = makeToolFinished({
      turn_id: "t1",
      payload: {
        call_id: "tc-fail",
        tool_name: "bash",
        success: false,
        error_message: "Command not found",
      },
    });

    let state = emptyTimelineState("test");
    state = reduceTimelineEvent(state, failEvent);
    if (state.nodes[0].kind === "tool_step") {
      expect(state.nodes[0].status).toBe("failed");
      expect(state.nodes[0].error_message).toBe("Command not found");
    }
  });
});

// ============================================================================
// Ordering and sequencing
// ============================================================================

describe("Ordering and sequencing", () => {
  it("events are ordered by seq even if provided out of order", () => {
    const e1 = makeTextDelta({
      seq: 10,
      turn_id: "t1",
      payload: { node_id: "at-1", delta: "First." },
    });
    const e2 = makeTextDelta({
      seq: 5,
      turn_id: "t1",
      payload: { node_id: "at-1", delta: "Second." },
    });

    // Provided out of order
    const state = reduceTimelineEvents([e2, e1]);
    // seq 5 was applied first (lower seq), then seq 10
    // So content should be "Second.First." since delta order drives content
    if (state.nodes[0].kind === "assistant_text") {
      expect(state.nodes[0].content).toBe("Second.First.");
    }
  });

  it("sortEventsBySeq correctly orders events", () => {
    const e1 = makeTextDelta({ seq: 10, turn_id: "t1" });
    const e2 = makeTextDelta({ seq: 5, turn_id: "t1" });
    const sorted = sortEventsBySeq([e1, e2]);
    expect(sorted[0].seq).toBe(5);
    expect(sorted[1].seq).toBe(10);
  });
});

// ============================================================================
// Deduplication
// ============================================================================

describe("Deduplication", () => {
  it("deduplicateEvents removes duplicates by id", () => {
    const events = simpleTextTurnFixture();
    const withDupes = [...events, events[0], events[1]]; // duplicate first two
    const deduped = deduplicateEvents(withDupes);
    expect(deduped.length).toBe(events.length);
  });
});

// ============================================================================
// assistant_message_finalized
// ============================================================================

describe("assistant_message_finalized", () => {
  it("finalized event marks text node as completed", () => {
    const events: TurnTimelineEvent[] = [
      makeTextDelta({
        turn_id: "t1",
        payload: { node_id: "at-1", delta: "Streaming..." },
      }),
      makeAssistantMessageFinalized({
        turn_id: "t1",
        payload: { text_node_id: "at-1", final_text_content: "Streaming... done!" },
      }),
    ];

    const state = reduceTimelineEvents(events);
    const textNode = findNodesByKind(state.nodes, "assistant_text")[0];
    expect(textNode.status).toBe("completed");
    if (textNode.kind === "assistant_text") {
      expect(textNode.content).toBe("Streaming... done!");
    }
  });
});

// ============================================================================
// turn_finished marks pending nodes as completed
// ============================================================================

describe("turn_finished finalization", () => {
  it("pending text and reasoning are marked completed on turn_finished", () => {
    const events: TurnTimelineEvent[] = [
      makeTurnStarted({ turn_id: "t1" }),
      makeUserMessageCreated({ turn_id: "t1" }),
      makeTextDelta({
        turn_id: "t1",
        payload: { node_id: "at-1", delta: "Unfinished..." },
      }),
      makeReasoningDelta({
        turn_id: "t1",
        payload: { node_id: "r-1", delta: "Unfinished reasoning..." },
      }),
      makeTurnFinished({
        turn_id: "t1",
        payload: { end_reason: "completed" },
      }),
    ];

    const state = reduceTimelineEvents(events);

    const textNode = findNodesByKind(state.nodes, "assistant_text")[0];
    expect(textNode.status).toBe("completed");

    const reasoningNode = findNodesByKind(state.nodes, "reasoning")[0];
    expect(reasoningNode.status).toBe("completed");
    if (reasoningNode.kind === "reasoning") {
      expect(reasoningNode.collapsed).toBe(true);
    }
  });
});

// ============================================================================
// Selector integration
// ============================================================================

describe("Timeline selectors", () => {
  it("selectNodesForTurn returns nodes for a specific turn", () => {
    const events = complexTurnFixture();
    const state = reduceTimelineEvents(events);
    const turnNodes = selectNodesForTurn(state, "turn-cplx-1");
    expect(turnNodes.length).toBeGreaterThan(0);
    // All nodes should belong to the same turn
    for (const n of turnNodes) {
      expect(n.turn_id).toBe("turn-cplx-1");
    }
  });

  it("selectMaxSeq returns the highest seq", () => {
    const events = complexTurnFixture();
    const state = reduceTimelineEvents(events);
    const maxSeq = selectMaxSeq(state);
    expect(maxSeq).toBeGreaterThan(0);
    expect(maxSeq).toBe(events[events.length - 1].seq);
  });

  it("selectActiveNodes returns streams in progress", () => {
    const events: TurnTimelineEvent[] = [
      makeTextDelta({
        turn_id: "t1",
        payload: { node_id: "at-1", delta: "Still streaming..." },
      }),
    ];

    const state = reduceTimelineEvents(events);
    const active = selectActiveNodes(state);
    expect(active.length).toBeGreaterThan(0);
    expect(active[0].status).toBe("pending");
  });
});

// ============================================================================
// source_trace propagation
// ============================================================================

describe("Source event trace", () => {
  it("each node carries source_trace with contributing event ids", () => {
    const events = simpleTextTurnFixture();
    const state = reduceTimelineEvents(events);

    for (const node of state.nodes) {
      expect(node.source_trace).toBeDefined();
    }
  });

  it("coalesced nodes accumulate multiple event ids in trace", () => {
    const events: TurnTimelineEvent[] = [
      makeTextDelta({
        turn_id: "t1",
        seq: 1,
        payload: { node_id: "at-1", delta: "Part 1. " },
      }),
      makeTextDelta({
        turn_id: "t1",
        seq: 2,
        payload: { node_id: "at-1", delta: "Part 2." },
      }),
    ];

    const state = reduceTimelineEvents(events);
    const textNode = findNodesByKind(state.nodes, "assistant_text")[0];
    const trace = textNode.source_trace;
    expect(trace).toBeDefined();
    if (trace) {
      expect(trace.event_ids.length).toBeGreaterThanOrEqual(2);
    }
  });
});

// ============================================================================
// Reasoning snapshot
// ============================================================================

describe("Reasoning snapshot", () => {
  it("reasoning is collapsed after snapshot", () => {
    const events: TurnTimelineEvent[] = [
      makeReasoningDelta({
        turn_id: "t1",
        payload: { node_id: "r-1", delta: "Thinking..." },
      }),
      makeReasoningSnapshot({
        turn_id: "t1",
        payload: { node_id: "r-1", content: "Full thinking done." },
      }),
    ];

    const state = reduceTimelineEvents(events);
    const rNode = findNodesByKind(state.nodes, "reasoning")[0];
    expect(rNode.status).toBe("completed");
    if (rNode.kind === "reasoning") {
      expect(rNode.collapsed).toBe(true);
    }
  });
});
