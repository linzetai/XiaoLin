// Timeline store — Zustand store holding TimelineState per session.
//
// Both live WebSocket delivery and history replay feed events through the
// same reducer, producing TurnDisplayNode[] for the UI to render.

import { create } from "zustand";
import type {
  TurnTimelineEvent,
  TimelineState,
  TurnDisplayNode,
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
  /** Per-session timeline state. */
  states: Record<string, TimelineState>;
  /** Per-session last seen sequence (for reconnect recovery). */
  lastSeenSeq: Record<string, number>;

  /** Add a single timeline event for a session, reducing it into state. */
  addEvent: (sessionId: string, event: TurnTimelineEvent) => void;
  /** Bulk-load timeline events (for initial load or replay). */
  loadEvents: (sessionId: string, events: TurnTimelineEvent[]) => void;
  /** Replace state with materialized display nodes (for full reload). */
  loadNodes: (sessionId: string, nodes: TurnDisplayNode[], maxSeq?: number) => void;
  /** Initialize state for a new session. */
  initSession: (sessionId: string) => void;
  /** Clean up state for a closed session. */
  cleanupSession: (sessionId: string) => void;
  /** Update the last seen sequence for a session (reconnect tracking). */
  setLastSeenSeq: (sessionId: string, seq: number) => void;
}

// ============================================================================
// Store
// ============================================================================

export const useTimelineStore = create<TimelineStore>((set) => ({
  states: {},
  lastSeenSeq: {},

  initSession: (sessionId) => {
    set((state) => {
      if (state.states[sessionId]) return state;
      return {
        states: {
          ...state.states,
          [sessionId]: emptyTimelineState(sessionId),
        },
      };
    });
  },

  cleanupSession: (sessionId) => {
    set((state) => {
      const { [sessionId]: _, ...states } = state.states;
      const { [sessionId]: __, ...lastSeenSeq } = state.lastSeenSeq;
      return { states, lastSeenSeq };
    });
  },

  addEvent: (sessionId, event) => {
    set((state) => {
      const current = state.states[sessionId] ?? emptyTimelineState(sessionId);
      const updated = reduceTimelineEvent(current, event);
      return {
        states: { ...state.states, [sessionId]: updated },
        lastSeenSeq: {
          ...state.lastSeenSeq,
          [sessionId]: Math.max(
            state.lastSeenSeq[sessionId] ?? 0,
            event.seq,
          ),
        },
      };
    });
  },

  loadEvents: (sessionId, events) => {
    set((state) => {
      const updated = reduceTimelineEvents(events);
      const incomingMaxSeq = events.reduce(
        (max, event) => Math.max(max, event.seq),
        0,
      );
      // Merge with any existing events (dedup by seq)
      const current = state.states[sessionId];
      if (current && current.events.length > 0) {
        const existingIds = new Set(current.events.map((e) => e.id));
        const newEvents = events.filter((e) => !existingIds.has(e.id));
        if (newEvents.length === 0) {
          if (incomingMaxSeq <= (state.lastSeenSeq[sessionId] ?? 0)) return state;
          return {
            lastSeenSeq: {
              ...state.lastSeenSeq,
              [sessionId]: incomingMaxSeq,
            },
          };
        }
        const allEvents = [...current.events, ...newEvents].sort(
          (a, b) => a.seq - b.seq,
        );
        const merged = reduceTimelineEvents(allEvents);
        return {
          states: { ...state.states, [sessionId]: merged },
          lastSeenSeq: {
            ...state.lastSeenSeq,
            [sessionId]: Math.max(
              state.lastSeenSeq[sessionId] ?? 0,
              merged.maxSeq,
            ),
          },
        };
      }
      return {
        states: { ...state.states, [sessionId]: updated },
        lastSeenSeq: {
          ...state.lastSeenSeq,
          [sessionId]: Math.max(
            state.lastSeenSeq[sessionId] ?? 0,
            updated.maxSeq,
          ),
        },
      };
    });
  },

  loadNodes: (sessionId, nodes, maxSeq) => {
    set((state) => {
      const current = state.states[sessionId] ?? emptyTimelineState(sessionId);
      return {
        states: {
          ...state.states,
          [sessionId]: {
            ...current,
            nodes,
          },
        },
        lastSeenSeq: maxSeq == null
          ? state.lastSeenSeq
          : {
              ...state.lastSeenSeq,
              [sessionId]: Math.max(state.lastSeenSeq[sessionId] ?? 0, maxSeq),
            },
      };
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
}));
