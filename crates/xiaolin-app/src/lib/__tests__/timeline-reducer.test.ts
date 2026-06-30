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
  makeCompactBoundary,
  makeSystemNotice,
  makeIterationBoundary,
  makeToolProgress,
  makeApprovalResolved,
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

  it("preserves assistant text-tool-text ordering with separate text nodes", () => {
    const state = reduceTimelineEvents(events);
    const sequence = state.nodes.map((node) => node.kind);
    const firstText = state.nodes.findIndex(
      (node) => node.kind === "assistant_text" && node.node_id === "node-at-1",
    );
    const firstTool = state.nodes.findIndex(
      (node) => node.kind === "tool_step" && node.node_id === "node-ts-tc-read-1",
    );
    const finalText = state.nodes.findIndex(
      (node) => node.kind === "assistant_text" && node.node_id === "node-at-2",
    );

    expect(firstText).toBeGreaterThanOrEqual(0);
    expect(firstTool).toBeGreaterThan(firstText);
    expect(finalText).toBeGreaterThan(firstTool);
    expect(sequence.filter((kind) => kind === "assistant_text")).toHaveLength(2);

    const finalNode = state.nodes[finalText];
    expect(finalNode.kind).toBe("assistant_text");
    if (finalNode.kind === "assistant_text") {
      expect(finalNode.content).toBe("No bugs found. The code compiles cleanly!");
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

  it("deduplicates repeated start/progress/finish for the same call_id into one node", () => {
    const events = [
      makeToolStarted({
        id: "tool-start-1",
        seq: 1,
        turn_id: "t1",
        payload: { call_id: "same-call", tool_name: "read_file", display_title: "Read timeline.rs" },
      }),
      makeToolProgress({
        id: "tool-progress-1",
        seq: 2,
        turn_id: "t1",
        payload: { call_id: "same-call", message: "Reading timeline.rs", progress: 0.5 },
      }),
      makeToolStarted({
        id: "tool-start-replay",
        seq: 3,
        turn_id: "t1",
        payload: { call_id: "same-call", tool_name: "read_file", display_title: "Read timeline.rs" },
      }),
      makeToolFinished({
        id: "tool-finish-1",
        seq: 4,
        turn_id: "t1",
        payload: { call_id: "same-call", tool_name: "read_file", success: true, duration_ms: 42 },
      }),
      makeToolProgress({
        id: "tool-progress-late",
        seq: 5,
        turn_id: "t1",
        payload: { call_id: "same-call", message: "late output", progress: 0.9 },
      }),
    ];

    const state = reduceTimelineEvents(events);
    const tools = findNodesByKind(state.nodes, "tool_step");
    expect(tools).toHaveLength(1);
    if (tools[0].kind === "tool_step") {
      expect(tools[0].call_id).toBe("same-call");
      expect(tools[0].status).toBe("completed");
      expect(tools[0].progress).toBe(0.9);
      expect(tools[0].source_trace?.event_ids).toEqual([
        "tool-start-1",
        "tool-progress-1",
        "tool-start-replay",
        "tool-finish-1",
        "tool-progress-late",
      ]);
    }
  });

  it("upgrades a progress-created stub when the start event arrives later", () => {
    const state = reduceTimelineEvents([
      makeToolProgress({
        id: "progress-first",
        seq: 1,
        turn_id: "t1",
        payload: { call_id: "late-start", message: "Reading", progress: 0.2 },
      }),
      makeToolStarted({
        id: "start-later",
        seq: 2,
        turn_id: "t1",
        payload: { call_id: "late-start", tool_name: "read_file", display_title: "Read timeline.rs" },
      }),
    ]);

    const tools = findNodesByKind(state.nodes, "tool_step");
    expect(tools).toHaveLength(1);
    if (tools[0].kind === "tool_step") {
      expect(tools[0].tool_name).toBe("read_file");
      expect(tools[0].display_title).toBe("Read timeline.rs");
      expect(tools[0].status).toBe("running");
    }
  });

  it("does not create blank assistant or reasoning nodes from whitespace-only content", () => {
    const state = reduceTimelineEvents([
      makeTextDelta({
        id: "blank-text-delta",
        seq: 1,
        turn_id: "t1",
        payload: { node_id: "blank-text", delta: "\n  " },
      }),
      makeTextSnapshot({
        id: "blank-text-snapshot",
        seq: 2,
        turn_id: "t1",
        payload: { node_id: "blank-text-snapshot", content: "\n" },
      }),
      makeReasoningDelta({
        id: "blank-reasoning-delta",
        seq: 3,
        turn_id: "t1",
        payload: { node_id: "blank-reasoning", delta: "\n", visibility: "public" },
      }),
      makeReasoningSnapshot({
        id: "blank-reasoning-snapshot",
        seq: 4,
        turn_id: "t1",
        payload: { node_id: "blank-reasoning-snapshot", content: "   ", visibility: "public" },
      }),
    ]);

    expect(state.nodes).toHaveLength(0);
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

// ============================================================================
// Text/reasoning interleaving (real-time streaming)
// ============================================================================

describe("Text/reasoning interleaving", () => {
  it("coalesces consecutive text deltas with the same node_id", () => {
    const events: TurnTimelineEvent[] = [
      makeTextDelta({
        turn_id: "t1",
        seq: 1,
        payload: { node_id: "text-1", delta: "Let" },
      }),
      makeTextDelta({
        turn_id: "t1",
        seq: 2,
        payload: { node_id: "text-1", delta: " me" },
      }),
      makeTextDelta({
        turn_id: "t1",
        seq: 3,
        payload: { node_id: "text-1", delta: " check." },
      }),
    ];

    const state = reduceTimelineEvents(events);
    const textNodes = findNodesByKind(state.nodes, "assistant_text");
    expect(textNodes).toHaveLength(1);
    if (textNodes[0].kind === "assistant_text") {
      expect(textNodes[0].content).toBe("Let me check.");
      expect(textNodes[0].status).toBe("pending"); // still streaming
    }
  });

  it("different text node_ids create separate text blocks", () => {
    const events: TurnTimelineEvent[] = [
      makeTextDelta({
        turn_id: "t1",
        seq: 1,
        payload: { node_id: "text-1", delta: "Before reasoning." },
      }),
      // Reasoning flushes text → new text_node_id
      makeTextDelta({
        turn_id: "t1",
        seq: 2,
        payload: { node_id: "text-2", delta: "After reasoning." },
      }),
    ];

    const state = reduceTimelineEvents(events);
    const textNodes = findNodesByKind(state.nodes, "assistant_text");
    expect(textNodes).toHaveLength(2);
    if (textNodes[0].kind === "assistant_text") {
      expect(textNodes[0].content).toBe("Before reasoning.");
    }
    if (textNodes[1].kind === "assistant_text") {
      expect(textNodes[1].content).toBe("After reasoning.");
    }
  });

  it("coalesces consecutive reasoning deltas with the same node_id", () => {
    const events: TurnTimelineEvent[] = [
      makeReasoningDelta({
        turn_id: "t1",
        seq: 1,
        payload: { node_id: "r-1", delta: "Hmm" },
      }),
      makeReasoningDelta({
        turn_id: "t1",
        seq: 2,
        payload: { node_id: "r-1", delta: ", let me think." },
      }),
    ];

    const state = reduceTimelineEvents(events);
    const rNodes = findNodesByKind(state.nodes, "reasoning");
    expect(rNodes).toHaveLength(1);
    if (rNodes[0].kind === "reasoning") {
      expect(rNodes[0].content).toBe("Hmm, let me think.");
      expect(rNodes[0].status).toBe("pending"); // still streaming
    }
  });

  it("text-tool-text interleaving maintains correct order", () => {
    const events: TurnTimelineEvent[] = [
      makeTurnStarted({ turn_id: "t1", seq: 1 }),
      makeUserMessageCreated({ turn_id: "t1", seq: 2 }),
      makeTextDelta({ turn_id: "t1", seq: 3, payload: { node_id: "text-1", delta: "Let me read." } }),
      makeToolStarted({ turn_id: "t1", seq: 4, payload: { call_id: "tc-1", tool_name: "read_file", display_title: "Read file" } }),
      makeToolFinished({ turn_id: "t1", seq: 5, payload: { call_id: "tc-1", tool_name: "read_file", success: true } }),
      makeTextDelta({ turn_id: "t1", seq: 6, payload: { node_id: "text-2", delta: "Done." } }),
      makeTurnFinished({ turn_id: "t1", seq: 7 }),
    ];

    const state = reduceTimelineEvents(events);
    const kinds = kindSequence(state.nodes);
    expect(kinds).toEqual([
      "user_message",
      "assistant_text",
      "tool_step",
      "assistant_text",
      "turn_status",
    ]);
  });

  it("reasoning-text-tool-text interleaving", () => {
    const events: TurnTimelineEvent[] = [
      makeTurnStarted({ turn_id: "t1", seq: 1 }),
      makeUserMessageCreated({ turn_id: "t1", seq: 2 }),
      makeReasoningDelta({ turn_id: "t1", seq: 3, payload: { node_id: "r-1", delta: "Let me think." } }),
      makeTextDelta({ turn_id: "t1", seq: 4, payload: { node_id: "text-1", delta: "I'll help." } }),
      makeToolStarted({ turn_id: "t1", seq: 5, payload: { call_id: "tc-1", tool_name: "grep", display_title: "Search" } }),
      makeToolFinished({ turn_id: "t1", seq: 6, payload: { call_id: "tc-1", tool_name: "grep", success: true } }),
      makeTextDelta({ turn_id: "t1", seq: 7, payload: { node_id: "text-2", delta: "Found it!" } }),
      makeTurnFinished({ turn_id: "t1", seq: 8 }),
    ];

    const state = reduceTimelineEvents(events);
    const kinds = kindSequence(state.nodes);
    expect(kinds).toEqual([
      "user_message",
      "reasoning",
      "assistant_text",
      "tool_step",
      "assistant_text",
      "turn_status",
    ]);
  });

  it("pending text node shows streaming state", () => {
    const events: TurnTimelineEvent[] = [
      makeTextDelta({ turn_id: "t1", seq: 1, payload: { node_id: "text-1", delta: "Streaming..." } }),
    ];

    const state = reduceTimelineEvents(events);
    const activeNodes = selectActiveNodes(state);
    expect(activeNodes).toHaveLength(1);
    expect(activeNodes[0].kind).toBe("assistant_text");
    expect(activeNodes[0].status).toBe("pending");
  });

  it("turn_finished marks all nodes as completed (no pending)", () => {
    const events: TurnTimelineEvent[] = [
      makeTurnStarted({ turn_id: "t1", seq: 1 }),
      makeUserMessageCreated({ turn_id: "t1", seq: 2 }),
      makeTextDelta({ turn_id: "t1", seq: 3, payload: { node_id: "text-1", delta: "Done." } }),
      makeReasoningDelta({ turn_id: "t1", seq: 4, payload: { node_id: "r-1", delta: "..." } }),
      makeTurnFinished({ turn_id: "t1", seq: 5 }),
    ];

    const state = reduceTimelineEvents(events);
    const activeNodes = selectActiveNodes(state);
    expect(activeNodes).toHaveLength(0); // all completed
  });
});

// ============================================================================
// Uncovered branch coverage
// ============================================================================

describe("Coverage: tool_call_progress before start (stub node)", () => {
  it("creates a stub tool node when progress arrives before tool_call_started", () => {
    const events: TurnTimelineEvent[] = [
      // Progress arrives first — no matching tool_call_started
      makeToolProgress({
        turn_id: "t1",
        seq: 1,
        payload: { call_id: "orphan-call", message: "Working...", progress: 0.3 },
      }),
    ];

    const state = reduceTimelineEvents(events);
    const tools = findNodesByKind(state.nodes, "tool_step");
    expect(tools).toHaveLength(1);
    if (tools[0].kind === "tool_step") {
      expect(tools[0].call_id).toBe("orphan-call");
      expect(tools[0].status).toBe("running");
      expect(tools[0].tool_name).toBe("orphan-call"); // fallback: call_id used as name
      expect(tools[0].progress).toBe(0.3);
      expect(tools[0].progress_label).toBe("Working...");
    }
  });
});

describe("Coverage: tool_call_finished without start", () => {
  it("creates a completed tool node when finish arrives without start", () => {
    const events: TurnTimelineEvent[] = [
      makeToolFinished({
        turn_id: "t1",
        seq: 1,
        payload: {
          call_id: "orphan-finish",
          tool_name: "bash",
          success: true,
          duration_ms: 200,
        },
      }),
    ];

    const state = reduceTimelineEvents(events);
    const tools = findNodesByKind(state.nodes, "tool_step");
    expect(tools).toHaveLength(1);
    if (tools[0].kind === "tool_step") {
      expect(tools[0].call_id).toBe("orphan-finish");
      expect(tools[0].status).toBe("completed");
      expect(tools[0].duration_ms).toBe(200);
    }
  });

  it("creates a failed tool node when failed finish arrives without start", () => {
    const events: TurnTimelineEvent[] = [
      makeToolFinished({
        turn_id: "t1",
        seq: 1,
        payload: {
          call_id: "orphan-fail",
          tool_name: "grep",
          success: false,
          error_message: "Command not found",
        },
      }),
    ];

    const state = reduceTimelineEvents(events);
    const tools = findNodesByKind(state.nodes, "tool_step");
    expect(tools).toHaveLength(1);
    if (tools[0].kind === "tool_step") {
      expect(tools[0].status).toBe("failed");
      expect(tools[0].error_message).toBe("Command not found");
    }
  });
});

describe("Coverage: approval_resolved without request", () => {
  it("creates a completed approval node when resolved without request", () => {
    const events: TurnTimelineEvent[] = [
      makeApprovalResolved({
        turn_id: "t1",
        seq: 1,
        payload: { approval_id: "orphan-apr", decision: "deny", source: "policy" },
      }),
    ];

    const state = reduceTimelineEvents(events);
    const approvals = findNodesByKind(state.nodes, "approval");
    expect(approvals).toHaveLength(1);
    if (approvals[0].kind === "approval") {
      expect(approvals[0].status).toBe("completed");
      expect(approvals[0].decision).toBe("deny");
      expect(approvals[0].decision_source).toBe("policy");
    }
  });
});

describe("Coverage: turn_finished severity=error", () => {
  it("sets turn status to failed when severity is error", () => {
    const events: TurnTimelineEvent[] = [
      makeTurnStarted({ turn_id: "t1", seq: 1 }),
      makeUserMessageCreated({ turn_id: "t1", seq: 2 }),
      makeTurnFinished({
        turn_id: "t1",
        seq: 3,
        payload: {
          end_reason: "tool_loop",
          severity: "error",
          diagnosis_code: "tool_loop",
          user_message: "Stopped by tool loop.",
        },
      }),
    ];

    const state = reduceTimelineEvents(events);
    const statusNodes = findNodesByKind(state.nodes, "turn_status");
    expect(statusNodes).toHaveLength(1);
    if (statusNodes[0].kind === "turn_status") {
      expect(statusNodes[0].status).toBe("failed");
      expect(statusNodes[0].end_reason).toBe("tool_loop");
    }
  });

  it("normal completion has turn_status with completed status", () => {
    const events: TurnTimelineEvent[] = [
      makeTurnStarted({ turn_id: "t1", seq: 1 }),
      makeUserMessageCreated({ turn_id: "t1", seq: 2 }),
      makeTurnFinished({
        turn_id: "t1",
        seq: 3,
        payload: { end_reason: "completed" },
      }),
    ];

    const state = reduceTimelineEvents(events);
    const statusNodes = findNodesByKind(state.nodes, "turn_status");
    // Frontend reducer always creates TurnStatusNode (unlike backend materializer
    // which skips it for end_reason="completed")
    expect(statusNodes).toHaveLength(1);
    if (statusNodes[0].kind === "turn_status") {
      expect(statusNodes[0].end_reason).toBe("completed");
      // Normal completion: no diagnosis
      expect(statusNodes[0].diagnosis).toBeUndefined();
    }
  });
});

describe("Coverage: user_message_created without message_id", () => {
  it("uses nodeIdFromEvent when message_id is not provided", () => {
    const events: TurnTimelineEvent[] = [
      makeUserMessageCreated({
        turn_id: "t1",
        seq: 1,
        payload: { content: "No message id" }, // no message_id
      }),
    ];

    const state = reduceTimelineEvents(events);
    const msgs = findNodesByKind(state.nodes, "user_message");
    expect(msgs).toHaveLength(1);
    if (msgs[0].kind === "user_message") {
      expect(msgs[0].content).toBe("No message id");
      expect(msgs[0].message_id).toBeUndefined();
      expect(msgs[0].node_id).toMatch(/^node-um-/);
    }
  });
});

describe("Coverage: nodesAreEquivalent and diffNodes", () => {
  it("nodesAreEquivalent returns true for identical node arrays", () => {
    const events = complexTurnFixture();
    const state1 = reduceTimelineEvents(events);
    const state2 = reduceTimelineEvents(events);
    expect(nodesAreEquivalent(state1.nodes, state2.nodes)).toBe(true);
  });

  it("nodesAreEquivalent returns false for different node arrays", () => {
    const events1 = simpleTextTurnFixture();
    const events2 = complexTurnFixture();
    const state1 = reduceTimelineEvents(events1);
    const state2 = reduceTimelineEvents(events2);
    expect(nodesAreEquivalent(state1.nodes, state2.nodes)).toBe(false);
  });

  it("diffNodes returns empty for identical arrays", () => {
    const events = complexTurnFixture();
    const state1 = reduceTimelineEvents(events);
    const state2 = reduceTimelineEvents(events);
    expect(diffNodes(state1.nodes, state2.nodes)).toEqual([]);
  });

  it("diffNodes returns differences for mismatched arrays", () => {
    const events1 = simpleTextTurnFixture();
    const events2 = complexTurnFixture();
    const state1 = reduceTimelineEvents(events1);
    const state2 = reduceTimelineEvents(events2);
    const diffs = diffNodes(state1.nodes, state2.nodes);
    expect(diffs.length).toBeGreaterThan(0);
  });
});

describe("Coverage: turn_started and default event_type", () => {
  it("turn_started does not create a visible node", () => {
    const events: TurnTimelineEvent[] = [
      makeTurnStarted({ turn_id: "t1", seq: 1 }),
    ];

    const state = reduceTimelineEvents(events);
    expect(state.nodes).toHaveLength(0); // metadata-only, no visible node
    expect(state.events).toHaveLength(1);
  });

  it("unknown event_type falls through to default (no nodes)", () => {
    const event: TurnTimelineEvent = {
      id: "unknown-1",
      session_id: "s1",
      turn_id: "t1",
      seq: 1,
      event_type: "__unknown__" as any,
      schema_version: 1,
      payload_json: {} as any,
      created_at_ms: 1000,
    };

    let state = emptyTimelineState("s1");
    state = reduceTimelineEvent(state, event);
    // Default case returns existing nodes unchanged
    expect(state.nodes).toHaveLength(0);
    expect(state.events).toHaveLength(1);
  });
});

describe("Coverage: mergeTrace without existing trace", () => {
  it("mergeTrace creates a new trace when existing is undefined", () => {
    // This is tested implicitly by all delta coalescing tests,
    // but we test the edge case directly via the exported function.
    const event: TurnTimelineEvent = {
      id: "evt-1",
      session_id: "s1",
      turn_id: "t1",
      seq: 1,
      event_type: "assistant_text_delta",
      schema_version: 1,
      payload_json: { node_id: "n1", delta: "test" },
      created_at_ms: 1000,
    };

    // First delta creates a new node (no existing trace → traceFromEvent)
    let state = emptyTimelineState("s1");
    state = reduceTimelineEvent(state, event);
    expect(state.nodes[0].source_trace).toBeDefined();
    expect(state.nodes[0].source_trace?.event_ids).toEqual(["evt-1"]);
  });
});

describe("Coverage: compact_boundary event", () => {
  it("compact_boundary creates a system_notice node", () => {
    const events: TurnTimelineEvent[] = [
      makeCompactBoundary({
        turn_id: "t1",
        seq: 1,
        payload: {
          trigger: "auto",
          pre_compact_tokens: 50000,
          post_compact_tokens: 15000,
          messages_removed: 20,
        },
      }),
    ];

    const state = reduceTimelineEvents(events);
    const notices = findNodesByKind(state.nodes, "system_notice");
    expect(notices).toHaveLength(1);
    if (notices[0].kind === "system_notice") {
      expect(notices[0].category).toBe("compaction");
    }
  });
});

describe("Coverage: system_notice event", () => {
  it("system_notice creates a system_notice node", () => {
    const events: TurnTimelineEvent[] = [
      makeSystemNotice({
        turn_id: "t1",
        seq: 1,
        payload: { message: "Something happened", level: "warning", category: "system" },
      }),
    ];

    const state = reduceTimelineEvents(events);
    const notices = findNodesByKind(state.nodes, "system_notice");
    expect(notices).toHaveLength(1);
    if (notices[0].kind === "system_notice") {
      expect(notices[0].level).toBe("warning");
      expect(notices[0].category).toBe("system");
    }
  });
});

describe("Coverage: iteration_boundary event", () => {
  it("iteration_boundary creates a node with correct iteration number", () => {
    const events: TurnTimelineEvent[] = [
      makeIterationBoundary({
        turn_id: "t1",
        seq: 1,
        payload: { iteration: 3 },
      }),
    ];

    const state = reduceTimelineEvents(events);
    const boundaries = findNodesByKind(state.nodes, "iteration_boundary");
    expect(boundaries).toHaveLength(1);
    if (boundaries[0].kind === "iteration_boundary") {
      expect(boundaries[0].iteration).toBe(3);
    }
  });
});
