import { beforeEach, describe, expect, it, vi } from "vitest";
import { useTimelineStore } from "../stores/timeline-store";
import { reduceTimelineEvents } from "../timeline/reducer";
import type { TurnTimelineEvent } from "../timeline/types";

vi.mock("../api", () => ({
  getTimelineMaxSeq: vi.fn(() => Promise.resolve({ session_id: "s1", max_seq: 3 })),
  getSessionDisplayNodes: vi.fn(() => Promise.resolve({
    session_id: "s1",
    nodes: [
      {
        kind: "system_notice",
        node_id: "node-sn-1",
        turn_id: "t1",
        status: "completed",
        created_at_ms: 1,
        updated_at_ms: 1,
        level: "info",
        category: "test",
        message: "loaded",
      },
    ],
    count: 1,
  })),
  getSessionTimeline: vi.fn(() => Promise.resolve({ session_id: "s1", events: [], count: 0 })),
}));

function event(seq: number, id = `evt-${seq}`): TurnTimelineEvent {
  return {
    id,
    session_id: "s1",
    turn_id: "t1",
    seq,
    event_type: "assistant_text_delta",
    schema_version: 2,
    payload_json: {
      node_id: "text-1",
      delta: String(seq),
    },
    created_at_ms: seq,
  };
}

describe("timeline store", () => {
  beforeEach(() => {
    useTimelineStore.setState({
      records: {},
      lastSeenSeq: {},
    });
  });

  it("ingestEvents from empty state creates fresh timeline", () => {
    useTimelineStore.getState().initSession("s2");
    useTimelineStore.getState().ingestEvents("s2", [event(1), event(2), event(3)]);

    const rec = useTimelineStore.getState().records.s2;
    expect(rec).toBeDefined();
    expect(rec.canonical.events).toHaveLength(3);
    expect(rec.canonical.maxSeq).toBe(3);
    expect(useTimelineStore.getState().lastSeenSeq.s2).toBe(3);
  });

  it("ingestEvent is idempotent (skips known event IDs)", () => {
    useTimelineStore.getState().initSession("s1");
    useTimelineStore.getState().ingestEvent("s1", event(1, "evt-1"));
    useTimelineStore.getState().ingestEvent("s1", event(1, "evt-1")); // duplicate

    const rec = useTimelineStore.getState().records.s1;
    expect(rec.canonical.events).toHaveLength(1);
  });

  it("ingestEvent advances lastSeenSeq", () => {
    useTimelineStore.getState().initSession("s1");
    useTimelineStore.getState().ingestEvent("s1", event(1));
    expect(useTimelineStore.getState().lastSeenSeq.s1).toBe(1);
    useTimelineStore.getState().ingestEvent("s1", event(2));
    expect(useTimelineStore.getState().lastSeenSeq.s1).toBe(2);
  });

  it("ingestEvent handles gap (stores in pending)", () => {
    useTimelineStore.getState().initSession("s1");
    // Ingest seq=5 (gap from 0)
    useTimelineStore.getState().ingestEvent("s1", event(5, "evt-5"));

    const rec = useTimelineStore.getState().records.s1;
    // Should be in pending, not applied
    expect(rec.lastContiguousSeq).toBe(0);
    expect(rec.pendingBySeq.has(5)).toBe(true);
    expect(rec.canonical.events).toHaveLength(0);
  });

  it("recalculateContiguous chains pending events", () => {
    useTimelineStore.getState().initSession("s1");
    // Ingest seq=3 (gap)
    useTimelineStore.getState().ingestEvent("s1", event(3, "evt-3"));
    // Fill gap with seq=1,2
    useTimelineStore.getState().fillGap("s1", [event(1, "evt-1"), event(2, "evt-2")]);

    const rec = useTimelineStore.getState().records.s1;
    expect(rec.lastContiguousSeq).toBe(3);
    expect(rec.canonical.events).toHaveLength(3);
  });

  it("initSession creates empty state for new session", () => {
    useTimelineStore.getState().initSession("new-session");

    const rec = useTimelineStore.getState().records["new-session"];
    expect(rec).toBeDefined();
    expect(rec.canonical.nodes).toHaveLength(0);
    expect(rec.canonical.events).toHaveLength(0);
    expect(rec.canonical.sessionId).toBe("new-session");
  });

  it("initSession is idempotent", () => {
    useTimelineStore.getState().initSession("s1");
    useTimelineStore.getState().ingestEvent("s1", event(1));
    const before = useTimelineStore.getState().records.s1.canonical.events.length;

    useTimelineStore.getState().initSession("s1"); // should be no-op
    const after = useTimelineStore.getState().records.s1.canonical.events.length;

    expect(after).toBe(before);
  });

  it("cleanupSession removes session state", () => {
    useTimelineStore.getState().initSession("s1");
    useTimelineStore.getState().ingestEvent("s1", event(1));
    expect(useTimelineStore.getState().records.s1).toBeDefined();

    useTimelineStore.getState().cleanupSession("s1");
    expect(useTimelineStore.getState().records.s1).toBeUndefined();
    expect(useTimelineStore.getState().lastSeenSeq.s1).toBeUndefined();
  });

  it("setLastSeenSeq advances the sequence", () => {
    useTimelineStore.getState().setLastSeenSeq("s1", 10);
    expect(useTimelineStore.getState().lastSeenSeq.s1).toBe(10);

    // Only advances, never goes backwards
    useTimelineStore.getState().setLastSeenSeq("s1", 5);
    expect(useTimelineStore.getState().lastSeenSeq.s1).toBe(10);
  });

  it("upsertOptimisticUser and removeOptimisticUser manage overlay", () => {
    useTimelineStore.getState().initSession("s1");
    useTimelineStore.getState().upsertOptimisticUser("s1", {
      clientMessageId: "client-1",
      localTurnId: "opt-turn-1",
      content: "Hello",
      createdAtMs: 1000,
      status: "sending",
    });

    let rec = useTimelineStore.getState().records.s1;
    expect(rec.optimisticUsers["client-1"]).toBeDefined();
    expect(rec.optimisticUsers["client-1"].content).toBe("Hello");

    useTimelineStore.getState().removeOptimisticUser("s1", "client-1");
    rec = useTimelineStore.getState().records.s1;
    expect(rec.optimisticUsers["client-1"]).toBeUndefined();
  });

  it("upsertToolOutputPatch and removeToolOutputPatch manage patches", () => {
    useTimelineStore.getState().initSession("s1");
    useTimelineStore.getState().upsertToolOutputPatch("s1", {
      callId: "tool-1",
      content: "partial output...",
      truncated: false,
      updatedAtMs: 2000,
    });

    let rec = useTimelineStore.getState().records.s1;
    expect(rec.toolOutputPatches["tool-1"]).toBeDefined();

    useTimelineStore.getState().removeToolOutputPatch("s1", "tool-1");
    rec = useTimelineStore.getState().records.s1;
    expect(rec.toolOutputPatches["tool-1"]).toBeUndefined();
  });

  it("replaceCanonicalTimeline atomically replaces state", () => {
    useTimelineStore.getState().initSession("s1");
    useTimelineStore.getState().ingestEvent("s1", event(1));

    // Replace with new state
    const newState = reduceTimelineEvents([event(10, "evt-10"), event(11, "evt-11")]);
    useTimelineStore.getState().replaceCanonicalTimeline("s1", newState);

    const rec = useTimelineStore.getState().records.s1;
    expect(rec.canonical.events).toHaveLength(2);
    expect(rec.canonical.maxSeq).toBe(11);
    expect(rec.lastContiguousSeq).toBe(11);
  });
});
