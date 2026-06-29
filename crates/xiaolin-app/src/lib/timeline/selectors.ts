// Timeline selectors.
//
// Pure functions that extract derived data from TimelineState.

import type {
  TimelineState,
  TurnDisplayNode,
  TurnDisplayNodeKind,
} from "./types";

/**
 * Select all display nodes.
 */
export function selectNodes(state: TimelineState): TurnDisplayNode[] {
  return state.nodes;
}

/**
 * Select display nodes for a specific turn, in creation order.
 */
export function selectNodesForTurn(
  state: TimelineState,
  turnId: string,
): TurnDisplayNode[] {
  const nodeIds = state.turnIndex[turnId] ?? [];
  const nodeMap = new Map(state.nodes.map((n) => [n.node_id, n]));
  return nodeIds
    .map((id) => nodeMap.get(id))
    .filter((n): n is TurnDisplayNode => n != null);
}

/**
 * Select nodes by kind.
 */
export function selectNodesByKind(
  state: TimelineState,
  kind: TurnDisplayNodeKind,
): TurnDisplayNode[] {
  return state.nodes.filter((n) => n.kind === kind);
}

/**
 * Select the maximum sequence number seen for a session.
 */
export function selectMaxSeq(state: TimelineState): number {
  return state.maxSeq;
}

/**
 * Select the last node in the timeline.
 */
export function selectLastNode(
  state: TimelineState,
): TurnDisplayNode | undefined {
  return state.nodes[state.nodes.length - 1];
}

/**
 * Select all turn ids present in the timeline, in order.
 */
export function selectTurnIds(state: TimelineState): string[] {
  const turnOrder: string[] = [];
  for (const event of state.events) {
    if (!turnOrder.includes(event.turn_id)) {
      turnOrder.push(event.turn_id);
    }
  }
  return turnOrder;
}

/**
 * Select active (non-completed) nodes — useful for streaming UI.
 */
export function selectActiveNodes(
  state: TimelineState,
): TurnDisplayNode[] {
  return state.nodes.filter(
    (n) => n.status === "pending" || n.status === "running",
  );
}

/**
 * Select the count of events by type.
 */
export function selectEventTypeCounts(
  state: TimelineState,
): Record<string, number> {
  const counts: Record<string, number> = {};
  for (const event of state.events) {
    counts[event.event_type] = (counts[event.event_type] ?? 0) + 1;
  }
  return counts;
}
