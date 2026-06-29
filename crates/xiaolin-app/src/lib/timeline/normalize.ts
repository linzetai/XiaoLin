// Normalization helpers for timeline state.
//
// These ensure that nodes and events produced by both live reduction and
// replay materialization can be compared deterministically.

import type { TurnDisplayNode, TurnTimelineEvent } from "./types";

/**
 * Normalize a TurnDisplayNode for comparison.
 *
 * - Sorts internal arrays (steps, event_ids)
 * - Strips non-deterministic fields (created_at_ms, updated_at_ms) for comparison
 * - Converts undefined optional fields to null for JSON comparison
 */
export function normalizeNodeForComparison(
  node: TurnDisplayNode,
): Record<string, unknown> {
  const cloned = JSON.parse(JSON.stringify(node)) as Record<string, unknown>;

  // Normalize source_trace event_ids ordering
  if (cloned.source_trace) {
    const trace = cloned.source_trace as Record<string, unknown>;
    if (Array.isArray(trace.event_ids)) {
      trace.event_ids = [...(trace.event_ids as string[])].sort();
    }
    // Strip seq values — non-deterministic in live mode
    delete trace.min_seq;
    delete trace.max_seq;
  }

  // Normalize tool_group steps ordering
  if (cloned.kind === "tool_group" && Array.isArray(cloned.steps)) {
    cloned.steps = (cloned.steps as TurnDisplayNode[]).map(
      (s) => normalizeNodeForComparison(s) as unknown as TurnDisplayNode,
    );
  }

  // Strip non-deterministic fields for comparison
  // (node_ids and timestamps are generated with counters/clocks that differ between live & replay)
  delete cloned.node_id;
  delete cloned.created_at_ms;
  delete cloned.updated_at_ms;

  return cloned;
}

/**
 * Compare two TurnDisplayNode arrays for structural equivalence.
 *
 * This is the golden-test equality check: live + replay nodes should
 * produce the same normalized representations.
 */
export function nodesAreEquivalent(
  live: TurnDisplayNode[],
  replay: TurnDisplayNode[],
): boolean {
  if (live.length !== replay.length) return false;

  const liveNorm = live.map(normalizeNodeForComparison);
  const replayNorm = replay.map(normalizeNodeForComparison);

  return JSON.stringify(liveNorm) === JSON.stringify(replayNorm);
}

/**
 * Diff two TurnDisplayNode arrays, returning a human-readable description
 * of differences (for test diagnostics).
 */
export function diffNodes(
  live: TurnDisplayNode[],
  replay: TurnDisplayNode[],
): string[] {
  const diffs: string[] = [];

  if (live.length !== replay.length) {
    diffs.push(
      `Count mismatch: live=${live.length} replay=${replay.length}`,
    );
  }

  const maxLen = Math.max(live.length, replay.length);
  for (let i = 0; i < maxLen; i++) {
    if (i >= live.length) {
      diffs.push(`[${i}] Missing in live: ${replay[i].kind} (${replay[i].node_id})`);
      continue;
    }
    if (i >= replay.length) {
      diffs.push(`[${i}] Missing in replay: ${live[i].kind} (${live[i].node_id})`);
      continue;
    }

    const l = normalizeNodeForComparison(live[i]);
    const r = normalizeNodeForComparison(replay[i]);

    if (JSON.stringify(l) !== JSON.stringify(r)) {
      diffs.push(
        `[${i}] Mismatch ${live[i].kind}/${replay[i].kind}: ` +
          `live=${live[i].node_id} replay=${replay[i].node_id}`,
      );
    }
  }

  return diffs;
}

/**
 * Ensure events are ordered by seq (for incoming events that may arrive
 * out of order in live delivery).
 */
export function sortEventsBySeq(
  events: TurnTimelineEvent[],
): TurnTimelineEvent[] {
  return [...events].sort((a, b) => a.seq - b.seq);
}

/**
 * Deduplicate events by id, keeping the first occurrence.
 */
export function deduplicateEvents(
  events: TurnTimelineEvent[],
): TurnTimelineEvent[] {
  const seen = new Set<string>();
  return events.filter((e) => {
    if (seen.has(e.id)) return false;
    seen.add(e.id);
    return true;
  });
}
