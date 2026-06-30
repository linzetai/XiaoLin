// Timeline store — Zustand store holding SessionTimelineRecord per session.
//
// Both live WebSocket delivery and history replay feed events through the
// same reducer, producing TurnDisplayNode[] for the UI to render.
//
// Phase 2: Gap-aware ingestion, source tracking, ephemeral overlays.

import { create } from "zustand";
import type {
  TurnTimelineEvent,
  TimelineState,
  ProvisionalUserMessage,
  ToolOutputPatch,
  TimelineSource,
  SessionTimelineRecord,
  ToolCallFinishedPayload,
  UserMessageCreatedPayload,
} from "../timeline/types";
import {
  emptyTimelineState,
  reduceTimelineEvent,
  reduceTimelineEvents,
} from "../timeline";

// ============================================================================
// Store types
// ============================================================================

export interface TimelineStore {
  /** Per-session timeline records (canonical + overlay + source). */
  records: Record<string, SessionTimelineRecord>;
  /** Per-session last seen sequence (for unread tracking). */
  lastSeenSeq: Record<string, number>;

  // ---- Canonical ingestion ----
  /** Ingest a single timeline event with gap awareness. */
  ingestEvent: (sessionId: string, event: TurnTimelineEvent) => void;
  /** Bulk-ingest timeline events. */
  ingestEvents: (sessionId: string, events: TurnTimelineEvent[]) => void;
  /** Atomically replace the canonical timeline state. */
  replaceCanonicalTimeline: (sessionId: string, state: TimelineState) => void;

  // ---- Source management ----
  setSource: (sessionId: string, source: TimelineSource) => void;
  startProbe: (sessionId: string) => void;
  completeProbe: (sessionId: string, snapshotEvents: TurnTimelineEvent[]) => void;

  // ---- Gap management ----
  recordPending: (sessionId: string, event: TurnTimelineEvent) => void;
  fillGap: (sessionId: string, events: TurnTimelineEvent[]) => void;
  recalculateContiguous: (sessionId: string) => void;

  // ---- Overlay management ----
  upsertOptimisticUser: (sessionId: string, user: ProvisionalUserMessage) => void;
  removeOptimisticUser: (sessionId: string, clientMessageId: string) => void;
  upsertToolOutputPatch: (sessionId: string, patch: ToolOutputPatch) => void;
  removeToolOutputPatch: (sessionId: string, callId: string) => void;

  // ---- Lifecycle ----
  /** Initialize a new session record. */
  initSession: (sessionId: string) => void;
  /** Clean up state for a closed session. */
  cleanupSession: (sessionId: string) => void;
  /** Update the last seen sequence for a session. */
  setLastSeenSeq: (sessionId: string, seq: number) => void;
}

// ============================================================================
// Helpers
// ============================================================================

function defaultRecord(sessionId: string): SessionTimelineRecord {
  return {
    source: "none",
    probeStartedAtMs: 0,
    probeCompletedAtMs: 0,
    canonical: emptyTimelineState(sessionId),
    knownById: new Map(),
    appliedEvents: [],
    pendingBySeq: new Map(),
    lastContiguousSeq: 0,
    gapFillInFlight: false,
    optimisticUsers: {},
    toolOutputPatches: {},
  };
}

function hasSeqConflict(rec: SessionTimelineRecord, event: TurnTimelineEvent): boolean {
  const applied = rec.appliedEvents.find((known) => known.seq === event.seq);
  if (applied && applied.id !== event.id) return true;
  const pending = rec.pendingBySeq.get(event.seq);
  return pending != null && pending.id !== event.id;
}

function applyOverlayEffects(
  rec: SessionTimelineRecord,
  event: TurnTimelineEvent,
): SessionTimelineRecord {
  if (event.event_type === "user_message_created") {
    const payload = event.payload_json as Partial<UserMessageCreatedPayload>;
    const clientMessageId = payload.client_message_id;
    if (clientMessageId && rec.optimisticUsers[clientMessageId]) {
      const { [clientMessageId]: _, ...optimisticUsers } = rec.optimisticUsers;
      return { ...rec, optimisticUsers };
    }
  }

  if (event.event_type === "tool_call_finished") {
    const payload = event.payload_json as Partial<ToolCallFinishedPayload>;
    const callId = payload.call_id;
    if (callId && rec.toolOutputPatches[callId]) {
      const { [callId]: _, ...toolOutputPatches } = rec.toolOutputPatches;
      return { ...rec, toolOutputPatches };
    }
  }

  return rec;
}

function applyOverlayEffectsForEvents(
  rec: SessionTimelineRecord,
  events: TurnTimelineEvent[],
): SessionTimelineRecord {
  return events.reduce(applyOverlayEffects, rec);
}

// ============================================================================
// Store
// ============================================================================

export const useTimelineStore = create<TimelineStore>((set, get) => ({
  records: {},
  lastSeenSeq: {},

  // ---- Lifecycle ----

  initSession: (sessionId) => {
    set((state) => {
      if (state.records[sessionId]) return state;
      return {
        records: {
          ...state.records,
          [sessionId]: defaultRecord(sessionId),
        },
      };
    });
  },

  cleanupSession: (sessionId) => {
    set((state) => {
      const { [sessionId]: _, ...records } = state.records;
      const { [sessionId]: __, ...lastSeenSeq } = state.lastSeenSeq;
      return { records, lastSeenSeq };
    });
  },

  setLastSeenSeq: (sessionId, seq) => {
    set((state) => ({
      lastSeenSeq: {
        ...state.lastSeenSeq,
        [sessionId]: Math.max(state.lastSeenSeq[sessionId] ?? 0, seq),
      },
    }));
  },

  // ---- Canonical ingestion ----

  ingestEvent: (sessionId, event) => {
    set((state) => {
      const rec = state.records[sessionId];
      if (!rec) return state;

      // Idempotent: skip if already known
      if (rec.knownById.has(event.id)) return state;
      if (hasSeqConflict(rec, event)) {
        console.warn("[timeline] Protocol violation: duplicate seq with different event id", {
          sessionId,
          seq: event.seq,
          eventId: event.id,
        });
        return state;
      }

      // Add to known set
      const newKnownById = new Map(rec.knownById);
      newKnownById.set(event.id, event);

      const newRec = { ...rec, knownById: newKnownById };

      // Check if contiguous
      if (event.seq === rec.lastContiguousSeq + 1) {
        // Contiguous — apply directly
        const newApplied = [...rec.appliedEvents, event];
        const newCanonical = reduceTimelineEvent(rec.canonical, event);
        newRec.appliedEvents = newApplied;
        newRec.canonical = newCanonical;
        newRec.lastContiguousSeq = event.seq;

        // After applying, check if pending events can now be chained
        // (recalculateContiguous logic inline)
        let nextSeq = event.seq + 1;
        let updatedRec = newRec;
        while (true) {
          const pending = updatedRec.pendingBySeq.get(nextSeq);
          if (!pending) break;
          const newPendingBySeq = new Map(updatedRec.pendingBySeq);
          newPendingBySeq.delete(nextSeq);
          updatedRec = {
            ...updatedRec,
            pendingBySeq: newPendingBySeq,
            appliedEvents: [...updatedRec.appliedEvents, pending],
            canonical: reduceTimelineEvent(updatedRec.canonical, pending),
            lastContiguousSeq: nextSeq,
          };
          updatedRec = applyOverlayEffects(updatedRec, pending);
          nextSeq++;
        }

        updatedRec = applyOverlayEffects(updatedRec, event);

        return {
          records: { ...state.records, [sessionId]: updatedRec },
          lastSeenSeq: {
            ...state.lastSeenSeq,
            [sessionId]: Math.max(state.lastSeenSeq[sessionId] ?? 0, event.seq),
          },
        };
      } else if (event.seq > rec.lastContiguousSeq + 1) {
        // Gap detected — store in pending
        const newPendingBySeq = new Map(rec.pendingBySeq);
        newPendingBySeq.set(event.seq, event);
        newRec.pendingBySeq = newPendingBySeq;

        // Trigger gap fill if not already in flight
        if (!rec.gapFillInFlight) {
          // Schedule async gap fill via setTimeout to avoid set-within-set
          setTimeout(() => {
            const currentRec = get().records[sessionId];
            if (currentRec && !currentRec.gapFillInFlight) {
              set((s) => {
                const r = s.records[sessionId];
                if (!r) return s;
                return {
                  records: {
                    ...s.records,
                    [sessionId]: { ...r, gapFillInFlight: true },
                  },
                };
              });
              // Gap fill will be triggered externally — set gapFillInFlight
              // The reconnect module or chat handler will fetch the missing events
            }
          }, 0);
        }

        return {
          records: { ...state.records, [sessionId]: newRec },
        };
      }
      // event.seq <= lastContiguousSeq: already covered, ignore

      return state;
    });
  },

  ingestEvents: (sessionId, events) => {
    if (events.length === 0) return;
    const sorted = [...events].sort((a, b) => a.seq - b.seq);
    for (const event of sorted) {
      get().ingestEvent(sessionId, event);
    }
  },

  replaceCanonicalTimeline: (sessionId, state) => {
    set((s) => {
      const rec = s.records[sessionId];
      if (!rec) return s;
      const nextRec = applyOverlayEffectsForEvents({
        ...rec,
        canonical: state,
        appliedEvents: state.events,
        lastContiguousSeq: state.maxSeq,
        // Rebuild knownById from the new events
        knownById: new Map(state.events.map((e) => [e.id, e])),
        // Clear pending — they're either in the new state or obsolete
        pendingBySeq: new Map(),
        gapFillInFlight: false,
      }, state.events);
      return {
        records: {
          ...s.records,
          [sessionId]: nextRec,
        },
        lastSeenSeq: {
          ...s.lastSeenSeq,
          [sessionId]: Math.max(s.lastSeenSeq[sessionId] ?? 0, state.maxSeq),
        },
      };
    });
  },

  // ---- Source management ----

  setSource: (sessionId, source) => {
    set((s) => {
      const rec = s.records[sessionId];
      if (!rec) return s;
      return {
        records: { ...s.records, [sessionId]: { ...rec, source } },
      };
    });
  },

  startProbe: (sessionId) => {
    set((s) => {
      const rec = s.records[sessionId] ?? defaultRecord(sessionId);
      return {
        records: {
          ...s.records,
          [sessionId]: {
            ...rec,
            source: "probing",
            probeStartedAtMs: Date.now(),
            probeCompletedAtMs: 0,
          },
        },
      };
    });
  },

  completeProbe: (sessionId, snapshotEvents) => {
    set((s) => {
      const rec = s.records[sessionId];
      if (!rec) return s;

      // Deduplicate: snapshot events + buffered WS events, sorted by seq
      const seenIds = new Set<string>();
      const merged: TurnTimelineEvent[] = [];
      for (const e of [...snapshotEvents, ...rec.appliedEvents]) {
        if (!seenIds.has(e.id)) {
          seenIds.add(e.id);
          merged.push(e);
        }
      }
      merged.sort((a, b) => a.seq - b.seq);

      const newState = reduceTimelineEvents(merged);

      // Determine source
      let newSource: TimelineSource = "authoritative";
      if (snapshotEvents.length === 0 && rec.appliedEvents.length === 0) {
        newSource = "legacy";
      }
      const nextRec = applyOverlayEffectsForEvents({
        ...rec,
        source: newSource,
        probeCompletedAtMs: Date.now(),
        canonical: newState,
        appliedEvents: merged,
        lastContiguousSeq: newState.maxSeq,
        knownById: new Map(merged.map((e) => [e.id, e])),
        pendingBySeq: new Map(),
        gapFillInFlight: false,
      }, merged);

      return {
        records: {
          ...s.records,
          [sessionId]: nextRec,
        },
      };
    });
  },

  // ---- Gap management ----

  recordPending: (sessionId, event) => {
    set((s) => {
      const rec = s.records[sessionId];
      if (!rec) return s;
      if (rec.knownById.has(event.id)) return s;
      if (hasSeqConflict(rec, event)) {
        console.warn("[timeline] Protocol violation: duplicate seq with different event id", {
          sessionId,
          seq: event.seq,
          eventId: event.id,
        });
        return s;
      }
      const newPendingBySeq = new Map(rec.pendingBySeq);
      newPendingBySeq.set(event.seq, event);
      const newKnownById = new Map(rec.knownById);
      newKnownById.set(event.id, event);
      return {
        records: {
          ...s.records,
          [sessionId]: { ...rec, pendingBySeq: newPendingBySeq, knownById: newKnownById },
        },
      };
    });
  },

  fillGap: (sessionId, events) => {
    if (events.length === 0) return;
    // Add all events to pending, then recalculate contiguous
    const store = get();
    for (const event of events) {
      store.recordPending(sessionId, event);
    }
    store.recalculateContiguous(sessionId);
  },

  recalculateContiguous: (sessionId) => {
    set((s) => {
      const rec = s.records[sessionId];
      if (!rec) return s;

      let updatedRec = { ...rec };
      let changed = false;

      while (true) {
        const nextSeq = updatedRec.lastContiguousSeq + 1;
        const pending = updatedRec.pendingBySeq.get(nextSeq);
        if (!pending) break;

        changed = true;
        const newPendingBySeq = new Map(updatedRec.pendingBySeq);
        newPendingBySeq.delete(nextSeq);

        updatedRec = {
          ...updatedRec,
          pendingBySeq: newPendingBySeq,
          appliedEvents: [...updatedRec.appliedEvents, pending],
          canonical: reduceTimelineEvent(updatedRec.canonical, pending),
          lastContiguousSeq: nextSeq,
        };
        updatedRec = applyOverlayEffects(updatedRec, pending);
      }

      if (!changed) return s;

      return {
        records: { ...s.records, [sessionId]: { ...updatedRec, gapFillInFlight: false } },
      };
    });
  },

  // ---- Overlay management ----

  upsertOptimisticUser: (sessionId, user) => {
    set((s) => {
      const rec = s.records[sessionId];
      if (!rec) return s;
      return {
        records: {
          ...s.records,
          [sessionId]: {
            ...rec,
            optimisticUsers: { ...rec.optimisticUsers, [user.clientMessageId]: user },
          },
        },
      };
    });
  },

  removeOptimisticUser: (sessionId, clientMessageId) => {
    set((s) => {
      const rec = s.records[sessionId];
      if (!rec) return s;
      const { [clientMessageId]: _, ...optimisticUsers } = rec.optimisticUsers;
      return {
        records: {
          ...s.records,
          [sessionId]: { ...rec, optimisticUsers },
        },
      };
    });
  },

  upsertToolOutputPatch: (sessionId, patch) => {
    set((s) => {
      const rec = s.records[sessionId];
      if (!rec) return s;
      return {
        records: {
          ...s.records,
          [sessionId]: {
            ...rec,
            toolOutputPatches: { ...rec.toolOutputPatches, [patch.callId]: patch },
          },
        },
      };
    });
  },

  removeToolOutputPatch: (sessionId, callId) => {
    set((s) => {
      const rec = s.records[sessionId];
      if (!rec) return s;
      const { [callId]: _, ...toolOutputPatches } = rec.toolOutputPatches;
      return {
        records: {
          ...s.records,
          [sessionId]: { ...rec, toolOutputPatches },
        },
      };
    });
  },
}));
