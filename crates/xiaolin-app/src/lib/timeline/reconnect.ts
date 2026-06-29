// Reconnect recovery for the canonical timeline store.
//
// On WebSocket reconnect, the client uses the last seen sequence to catch up
// on missed timeline events. If the gap is too large or the client state is
// suspect, it falls back to a full display-node reload.

import { useTimelineStore } from "../stores/timeline-store";
import * as api from "../api";
import type { TurnTimelineEvent, TurnDisplayNode } from "./types";

/** Maximum sequence gap before falling back to full reload. */
const MAX_RECONNECT_GAP = 500;

/**
 * Recover timeline state for a session after WebSocket reconnect.
 *
 * 1. Query the backend's max_seq
 * 2. Compare with local lastSeenSeq
 * 3. If gap is small, fetch & reduce incremental events
 * 4. If gap is large or state is missing, reload display nodes
 *
 * Returns true if recovery was successful (events or nodes were loaded).
 */
export async function recoverTimelineAfterReconnect(
  sessionId: string,
): Promise<boolean> {
  const store = useTimelineStore.getState();
  const lastSeen = store.lastSeenSeq[sessionId] ?? 0;

  try {
    // 1. Get max_seq from backend
    const maxSeqResp = await api.getTimelineMaxSeq(sessionId);
    const backendMax = maxSeqResp.max_seq ?? 0;

    // 2. No gap — nothing to do
    if (lastSeen >= backendMax) {
      return true;
    }

    // 3. Check gap size
    const gap = backendMax - lastSeen;
    if (gap > MAX_RECONNECT_GAP || lastSeen === 0) {
      // Large gap or no local state — fall back to full display-node reload
      const displayResp = await api.getSessionDisplayNodes(sessionId);
      if (displayResp.nodes && displayResp.nodes.length > 0) {
        const nodes = displayResp.nodes.map((raw) => ({
          ...raw,
          kind: (raw.kind as string) || "system_notice",
        })) as TurnDisplayNode[];
        useTimelineStore.getState().loadNodes(sessionId, nodes);
        return true;
      }

      // No display nodes — try raw events
      const tlResp = await api.getSessionTimeline(sessionId, undefined, 2000);
      if (tlResp.events && tlResp.events.length > 0) {
        useTimelineStore.getState().loadEvents(
          sessionId,
          tlResp.events as unknown as TurnTimelineEvent[],
        );
        return true;
      }

      return false;
    }

    // 4. Small gap — fetch incremental events
    const tlResp = await api.getSessionTimeline(sessionId, lastSeen, gap + 50);
    if (tlResp.events && tlResp.events.length > 0) {
      useTimelineStore.getState().loadEvents(
        sessionId,
        tlResp.events as unknown as TurnTimelineEvent[],
      );
    }

    return true;
  } catch {
    // Recovery failed — timeline data may not be available
    return false;
  }
}

/**
 * Initialize timeline state for an active session.
 * Called when the user opens or starts a chat.
 */
export async function initTimelineForSession(
  sessionId: string,
): Promise<void> {
  const store = useTimelineStore.getState();

  // Initialize if not already present
  if (!store.states[sessionId]) {
    store.initSession(sessionId);
  }

  // Try to load existing display nodes (for history sessions)
  try {
    const displayResp = await api.getSessionDisplayNodes(sessionId);
    if (displayResp.nodes && displayResp.nodes.length > 0) {
      const nodes = displayResp.nodes.map((raw) => ({
        ...raw,
        kind: (raw.kind as string) || "system_notice",
      })) as TurnDisplayNode[];
      useTimelineStore.getState().loadNodes(sessionId, nodes);
    }
  } catch {
    // Pre-change session or no timeline data — that's fine
  }
}
