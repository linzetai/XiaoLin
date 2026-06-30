// Timeline selectors.
//
// Pure functions that extract derived data from TimelineState.

import type {
  TimelineState,
  ProvisionalUserMessage,
  ToolOutputPatch,
  TurnDisplayNode,
  TurnDisplayNodeKind,
  UserMessageNode,
} from "./types";

// ============================================================================
// Turn grouping for Codex/ChatGPT-style message blocks
// ============================================================================

/**
 * A grouped turn for rendering as a message block.
 *
 * Each turn has at most one user message and zero or more assistant nodes
 * (text, reasoning, tool steps, tool groups, approvals, iteration boundaries,
 * turn status, system notices) in timeline order.
 */
export interface TurnGroup {
  /** Stable key for this contiguous rendered group. */
  groupId: string;
  turnId: string;
  /** The user message that started this turn, if any. */
  userMessageNode: UserMessageNode | null;
  /** All assistant-side nodes in timeline order. */
  assistantNodes: TurnDisplayNode[];
}

export interface TranscriptRenderModelInput {
  canonicalState: TimelineState;
  optimisticUsers: Record<string, ProvisionalUserMessage>;
  toolOutputPatches: Record<string, ToolOutputPatch>;
}

export interface TranscriptRenderModel {
  turnGroups: TurnGroup[];
  toolOutputPatches: Record<string, ToolOutputPatch>;
}

export function selectTranscriptRenderModel({
  canonicalState,
  optimisticUsers,
  toolOutputPatches,
}: TranscriptRenderModelInput): TranscriptRenderModel {
  const turnGroups = selectTurnGroups(canonicalState);
  const optimisticGroups = Object.values(optimisticUsers)
    .sort((a, b) => a.createdAtMs - b.createdAtMs)
    .map((user, index): TurnGroup => {
      const node: UserMessageNode = {
        kind: "user_message",
        node_id: `optimistic-user-${user.clientMessageId}`,
        turn_id: user.localTurnId,
        status: user.status === "failed" ? "failed" : "pending",
        created_at_ms: user.createdAtMs,
        updated_at_ms: user.createdAtMs,
        content: user.content,
        attachments: user.attachments,
      };
      return {
        groupId: `${user.localTurnId}:optimistic:${index}`,
        turnId: user.localTurnId,
        userMessageNode: node,
        assistantNodes: [],
      };
    });

  return {
    turnGroups: [...turnGroups, ...optimisticGroups],
    toolOutputPatches,
  };
}

/**
 * Partition flat TurnDisplayNode[] into turn-grouped message blocks.
 *
 * Nodes with kind "user_message" are extracted as the turn's user message.
 * All other nodes within the same contiguous turn segment are assistant-side
 * nodes. A turn that receives later detached/interleaved activity creates a
 * later continuation group, preserving the global timeline order instead of
 * moving new activity back to the turn's first appearance.
 */
export function selectTurnGroups(state: TimelineState): TurnGroup[] {
  const groups: TurnGroup[] = [];
  let current: TurnGroup | null = null;

  for (const node of state.nodes) {
    const needsNewGroup =
      current == null ||
      current.turnId !== node.turn_id ||
      (node.kind === "user_message" &&
        (current.userMessageNode != null || current.assistantNodes.length > 0));

    if (needsNewGroup) {
      current = {
        groupId: `${node.turn_id}:${groups.length}`,
        turnId: node.turn_id,
        userMessageNode: null,
        assistantNodes: [],
      };
      groups.push(current);
    }

    const group = current;
    if (group == null) {
      throw new Error("selectTurnGroups invariant violated: missing active group");
    }
    if (node.kind === "user_message") {
      group.userMessageNode = node;
    } else {
      group.assistantNodes.push(node);
    }
  }

  return groups;
}

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
