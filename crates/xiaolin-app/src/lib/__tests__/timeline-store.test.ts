import { beforeEach, describe, expect, it, vi } from "vitest";
import { useTimelineStore } from "../stores/timeline-store";
import { recoverTimelineAfterReconnect } from "../timeline/reconnect";
import type { TurnTimelineEvent, TurnDisplayNode } from "../timeline/types";

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
    schema_version: 1,
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
      states: {},
      lastSeenSeq: {},
    });
  });

  it("advances lastSeenSeq when merging incremental events into an existing timeline", () => {
    const store = useTimelineStore.getState();

    store.loadEvents("s1", [event(1), event(2)]);
    expect(useTimelineStore.getState().lastSeenSeq.s1).toBe(2);

    useTimelineStore.getState().loadEvents("s1", [event(3)]);
    const state = useTimelineStore.getState();

    expect(state.lastSeenSeq.s1).toBe(3);
    expect(state.states.s1.events.map((e) => e.seq)).toEqual([1, 2, 3]);
  });

  it("advances lastSeenSeq when duplicate incremental events are observed", () => {
    const store = useTimelineStore.getState();

    store.loadEvents("s1", [event(1)]);
    useTimelineStore.getState().loadEvents("s1", [event(1), event(2)]);

    expect(useTimelineStore.getState().lastSeenSeq.s1).toBe(2);
  });

  it("records max sequence when materialized display nodes are loaded", () => {
    const node: TurnDisplayNode = {
      kind: "system_notice",
      node_id: "node-sn-1",
      turn_id: "t1",
      status: "completed",
      created_at_ms: 1,
      updated_at_ms: 1,
      level: "info",
      category: "test",
      message: "loaded",
    };

    useTimelineStore.getState().loadNodes("s1", [node], 42);

    expect(useTimelineStore.getState().lastSeenSeq.s1).toBe(42);
  });

  it("records backend max sequence after reconnect display-node fallback", async () => {
    const recovered = await recoverTimelineAfterReconnect("s1");

    expect(recovered).toBe(true);
    expect(useTimelineStore.getState().lastSeenSeq.s1).toBe(3);
    expect(useTimelineStore.getState().states.s1.nodes).toHaveLength(1);
  });

  it("loadEvents from empty state creates fresh timeline", () => {
    useTimelineStore.getState().loadEvents("s2", [event(1), event(2), event(3)]);

    const state = useTimelineStore.getState();
    expect(state.states.s2).toBeDefined();
    expect(state.states.s2.events).toHaveLength(3);
    expect(state.states.s2.maxSeq).toBe(3);
    expect(state.lastSeenSeq.s2).toBe(3);
  });

  it("loadEvents with all already-seen events only updates seq", () => {
    // First load
    useTimelineStore.getState().loadEvents("s1", [event(1), event(2)]);

    // Second load — all events already exist
    useTimelineStore.getState().loadEvents("s1", [event(1), event(2)]);

    const state = useTimelineStore.getState();
    expect(state.states.s1.events).toHaveLength(2); // no duplicates
    expect(state.lastSeenSeq.s1).toBe(2);
  });

  it("loadEvents merges new events even with lower seq (IDs determine newness)", () => {
    useTimelineStore.getState().loadEvents("s1", [event(5)]);

    // Load events with different IDs but lower seq — they're still new
    useTimelineStore.getState().loadEvents("s1", [event(1), event(2)]);

    const state = useTimelineStore.getState();
    // All 3 events are present because they have different IDs
    expect(state.states.s1.events).toHaveLength(3);
    expect(state.lastSeenSeq.s1).toBe(5); // max seq is 5
  });

  it("initSession creates empty state for new session", () => {
    useTimelineStore.getState().initSession("new-session");

    const state = useTimelineStore.getState();
    expect(state.states["new-session"]).toBeDefined();
    expect(state.states["new-session"].nodes).toHaveLength(0);
    expect(state.states["new-session"].events).toHaveLength(0);
    expect(state.states["new-session"].sessionId).toBe("new-session");
  });

  it("initSession is idempotent (does not overwrite existing)", () => {
    useTimelineStore.getState().loadEvents("s1", [event(1)]);
    const before = useTimelineStore.getState().states.s1.events.length;

    useTimelineStore.getState().initSession("s1"); // should be no-op
    const after = useTimelineStore.getState().states.s1.events.length;

    expect(after).toBe(before);
  });

  it("cleanupSession removes session state", () => {
    useTimelineStore.getState().loadEvents("s1", [event(1)]);
    expect(useTimelineStore.getState().states.s1).toBeDefined();

    useTimelineStore.getState().cleanupSession("s1");
    expect(useTimelineStore.getState().states.s1).toBeUndefined();
    expect(useTimelineStore.getState().lastSeenSeq.s1).toBeUndefined();
  });

  it("addEvent reduces a single event into state", () => {
    useTimelineStore.getState().addEvent("s1", event(1));

    const state = useTimelineStore.getState();
    expect(state.states.s1.events).toHaveLength(1);
    expect(state.states.s1.maxSeq).toBe(1);
    expect(state.lastSeenSeq.s1).toBe(1);
  });

  it("setLastSeenSeq advances the sequence for a session", () => {
    useTimelineStore.getState().setLastSeenSeq("s1", 10);
    expect(useTimelineStore.getState().lastSeenSeq.s1).toBe(10);

    // Only advances, never goes backwards
    useTimelineStore.getState().setLastSeenSeq("s1", 5);
    expect(useTimelineStore.getState().lastSeenSeq.s1).toBe(10);
  });

  it("reconnect recovery syncs when backend has data but client doesn't", async () => {
    const { getTimelineMaxSeq, getSessionDisplayNodes } = await import("../api");
    // Client has no state (lastSeen=0), backend has max_seq=5
    vi.mocked(getTimelineMaxSeq).mockResolvedValueOnce({ session_id: "new", max_seq: 5 });
    vi.mocked(getSessionDisplayNodes).mockResolvedValueOnce({
      session_id: "new",
      nodes: [{ kind: "user_message", node_id: "n1", turn_id: "t1", status: "completed", created_at_ms: 1, updated_at_ms: 1, content: "Hi" }] as unknown as Array<Record<string, unknown>>,
      count: 1,
    });

    const recovered = await recoverTimelineAfterReconnect("new");
    expect(recovered).toBe(true);
    expect(useTimelineStore.getState().states["new"]).toBeDefined();
    expect(useTimelineStore.getState().states["new"].nodes).toHaveLength(1);
  });

  it("reconnect recovery returns true when already in sync (no gap)", async () => {
    // First prime the store with a known lastSeen value
    useTimelineStore.getState().setLastSeenSeq("synced", 3);

    const { getTimelineMaxSeq } = await import("../api");
    vi.mocked(getTimelineMaxSeq).mockResolvedValueOnce({ session_id: "synced", max_seq: 3 });

    const recovered = await recoverTimelineAfterReconnect("synced");
    expect(recovered).toBe(true); // In sync, nothing to do
  });
});
