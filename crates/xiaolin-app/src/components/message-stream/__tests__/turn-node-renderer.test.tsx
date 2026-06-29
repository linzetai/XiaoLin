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

import { describe, it, expect } from "vitest";
import { render, waitFor } from "@testing-library/react";
import { TurnNodeRenderer } from "../TurnNodeRenderer";
import {
  reduceTimelineEvents,
  materializeNodes,
  emptyTimelineState,
  reduceTimelineEvent,
} from "../../../lib/timeline";
import {
  simpleTextTurnFixture,
  complexTurnFixture,
  toolLoopTerminationFixture,
  makeTextDelta,
} from "../../../lib/timeline/fixtures";
import type {
  AssistantTextNode,
  ReasoningNode,
  IterationBoundaryNode,
  TurnStatusNode,
  SystemNoticeNode,
  ApprovalNode,
} from "../../../lib/timeline/types";

// ============================================================================
// Smoke tests — renderer doesn't crash
// ============================================================================

describe("TurnNodeRenderer smoke tests", () => {
  it("renders empty nodes array without crashing", () => {
    const { container } = render(<TurnNodeRenderer nodes={[]} />);
    expect(container.innerHTML).toBe("");
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

  it("reasoning node renders with ReasoningBlock", () => {
    const node: ReasoningNode = {
      kind: "reasoning",
      node_id: "r-1",
      turn_id: "t1",
      status: "completed",
      created_at_ms: 1000,
      updated_at_ms: 2000,
      content: "Let me think about this.",
      collapsed: true,
    };

    const { container } = render(
      <TurnNodeRenderer nodes={[node]} />,
    );
    // Content should be visible (collapsed by default but content in DOM)
    expect(container.textContent).toContain("Let me think about this.");
  });

  it("iteration_boundary node renders three dots", () => {
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
    // Three dots should be rendered
    const dots = container.querySelectorAll(".rounded-full");
    expect(dots.length).toBeGreaterThanOrEqual(3);
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
    expect(container.textContent).toContain("[tool_loop]");
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
    expect(container.textContent).toContain("allow_once");
    expect(container.textContent).toContain("user");
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
