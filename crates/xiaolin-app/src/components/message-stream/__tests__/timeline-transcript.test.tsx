/**
 * @vitest-environment jsdom
 */

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render, waitFor } from "@testing-library/react";
import { TimelineTranscript } from "../TimelineTranscript";
import { useTimelineStore } from "../../../lib/stores/timeline-store";
import { emptyTimelineState } from "../../../lib/timeline";
import type { TurnDisplayNode, TimelineState } from "../../../lib/timeline";

const measureMock = vi.fn();

vi.mock("@tanstack/react-virtual", () => ({
  useVirtualizer: vi.fn(() => ({
    getVirtualItems: () => [],
    getTotalSize: () => 0,
    measureElement: vi.fn(),
    measure: measureMock,
    isAtEnd: () => true,
    scrollToEnd: vi.fn(),
  })),
}));

function userNode(content: string): TurnDisplayNode {
  return {
    kind: "user_message",
    node_id: "user-1",
    turn_id: "turn-1",
    status: "completed",
    created_at_ms: 1000,
    updated_at_ms: 1000,
    content,
  };
}

function textNode(content: string): TurnDisplayNode {
  return {
    kind: "assistant_text",
    node_id: "answer-1",
    turn_id: "turn-1",
    status: "completed",
    created_at_ms: 1100,
    updated_at_ms: 1100,
    content,
  };
}

function loadNodesForTest(sessionId: string, nodes: TurnDisplayNode[]) {
  const store = useTimelineStore.getState();
  store.initSession(sessionId);
  const state: TimelineState = {
    ...emptyTimelineState(sessionId),
    nodes,
  };
  store.replaceCanonicalTimeline(sessionId, state);
}

describe("TimelineTranscript regression coverage", () => {
  beforeEach(() => {
    measureMock.mockClear();
    useTimelineStore.setState({ records: {}, lastSeenSeq: {} });
  });

  afterEach(() => {
    useTimelineStore.setState({ records: {}, lastSeenSeq: {} });
  });

  it("renders timeline content with a static fallback when virtual items are temporarily empty", async () => {
    loadNodesForTest("session-fallback", [
      userNode("review下代码"),
      textNode("可以，我来审查。"),
    ]);

    const { container } = render(
      <TimelineTranscript sessionId="session-fallback" />,
    );

    expect(container.querySelector("[data-virtualizer-fallback='true']")).toBeTruthy();
    expect(container.textContent).toContain("review下代码");
    await waitFor(() => {
      expect(container.textContent).toContain("可以，我来审查。");
    });
  });

  it("remeasures when switching to a session that already has timeline nodes", async () => {
    const { rerender } = render(<TimelineTranscript sessionId="empty-session" />);

    loadNodesForTest("session-with-nodes", [
      userNode("切回这个会话"),
      textNode("不会白屏。"),
    ]);

    rerender(<TimelineTranscript sessionId="session-with-nodes" />);

    await waitFor(() => {
      expect(measureMock).toHaveBeenCalled();
    });
    expect(document.body.textContent).toContain("切回这个会话");
    expect(document.body.textContent).toContain("不会白屏。");
  });

  it("renders a live connecting state while waiting for the first assistant event", async () => {
    const store = useTimelineStore.getState();
    store.initSession("session-pending");
    store.upsertOptimisticUser("session-pending", {
      clientMessageId: "client-1",
      localTurnId: "optimistic-turn-client-1",
      content: "review下代码",
      attachments: [],
      createdAtMs: 1000,
      status: "sending",
    });

    const { container } = render(
      <TimelineTranscript sessionId="session-pending" isLive />,
    );

    await waitFor(() => {
      expect(container.textContent).toContain("review下代码");
      expect(container.textContent).toContain("连接中");
    });
  });
});
