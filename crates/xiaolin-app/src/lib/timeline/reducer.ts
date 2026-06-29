// Canonical turn timeline reducer.
//
// Reduces TurnTimelineEvent[] into TimelineState containing materialized
// TurnDisplayNode[].  Both live WebSocket delivery and history replay use
// the same reducer semantics (Decision D3).

import type {
  TurnTimelineEvent,
  TimelineState,
  TurnDisplayNode,
  UserMessageNode,
  AssistantTextNode,
  ReasoningNode,
  ToolStepNode,
  ApprovalNode,
  IterationBoundaryNode,
  TurnStatusNode,
  SystemNoticeNode,
  SourceEventTrace,
  NodeStatus,
  UserMessageCreatedPayload,
  AssistantTextDeltaPayload,
  AssistantTextSnapshotPayload,
  ReasoningDeltaPayload,
  ReasoningSnapshotPayload,
  ToolCallStartedPayload,
  ToolCallProgressPayload,
  ToolCallFinishedPayload,
  ApprovalRequestedPayload,
  ApprovalResolvedPayload,
  IterationBoundaryPayload,
  AssistantMessageFinalizedPayload,
  TurnFinishedPayload,
  CompactBoundaryPayload,
  SystemNoticePayload,
} from "./types";
import { emptyTimelineState } from "./types";

// ============================================================================
// Helpers
// ============================================================================

let _nodeCounter = 0;

/** Generate a stable node id. */
function nextNodeId(prefix: string): string {
  _nodeCounter += 1;
  return `node-${prefix}-${_nodeCounter}-${Date.now().toString(36)}`;
}

/** Build a SourceEventTrace from an event. */
function traceFromEvent(event: TurnTimelineEvent): SourceEventTrace {
  return {
    event_ids: [event.id],
    min_seq: event.seq,
    max_seq: event.seq,
  };
}

/** Merge two source traces (for coalesced events). */
function mergeTrace(
  existing: SourceEventTrace | undefined,
  event: TurnTimelineEvent,
): SourceEventTrace {
  if (!existing) return traceFromEvent(event);
  return {
    event_ids: [...existing.event_ids, event.id],
    min_seq: Math.min(existing.min_seq ?? event.seq, event.seq),
    max_seq: Math.max(existing.max_seq ?? event.seq, event.seq),
  };
}

/** Determine NodeStatus from tool finish success. */
function successStatus(success: boolean): NodeStatus {
  return success ? "completed" : "failed";
}

/** Mark a completed node's status. */
function completeStatus(status: NodeStatus): NodeStatus {
  if (status === "running" || status === "pending") return "completed";
  return status;
}

/** Safe JSON parse for payload. */
function parsePayload<T>(event: TurnTimelineEvent): T {
  return event.payload_json as unknown as T;
}

// ============================================================================
// Reducer entry point
// ============================================================================

/**
 * Apply an array of timeline events to produce a new TimelineState.
 *
 * Events are applied in order. Delta events (assistant_text_delta,
 * reasoning_delta) are coalesced into existing nodes when they target
 * the same node_id. Empty deltas are ignored.
 */
export function reduceTimelineEvents(
  events: TurnTimelineEvent[],
): TimelineState {
  const sorted = [...events].sort((a, b) => a.seq - b.seq);
  const sessionId = sorted[0]?.session_id ?? "";
  let state = emptyTimelineState(sessionId);

  for (const event of sorted) {
    state = reduceTimelineEvent(state, event);
  }

  return state;
}

/**
 * Apply a single timeline event to the state, returning the new state.
 */
export function reduceTimelineEvent(
  state: TimelineState,
  event: TurnTimelineEvent,
): TimelineState {
  // Idempotent: skip if we already have this event id
  if (state.events.some((e) => e.id === event.id)) {
    return state;
  }

  const newEvents = [...state.events, event];
  const newMaxSeq = Math.max(state.maxSeq, event.seq);

  // Update event trace for the node
  const trackEvent = (nodeId: string) => {
    const traces = { ...state.eventTraces };
    traces[nodeId] = [...(traces[nodeId] ?? []), event.id];
    return traces;
  };

  let newNodes: TurnDisplayNode[];
  let newEventTraces = state.eventTraces;
  const newTurnIndex = { ...state.turnIndex };

  switch (event.event_type) {
    case "turn_started": {
      // Turn start does not produce a visible node — it's metadata.
      newNodes = state.nodes;
      break;
    }

    case "user_message_created": {
      const payload = parsePayload<UserMessageCreatedPayload>(event);
      const nodeId = payload.message_id
        ? `node-um-${payload.message_id}`
        : nextNodeId("um");
      const node: UserMessageNode = {
        kind: "user_message",
        node_id: nodeId,
        turn_id: event.turn_id,
        status: "completed",
        created_at_ms: event.created_at_ms,
        updated_at_ms: event.created_at_ms,
        content: payload.content,
        message_id: payload.message_id,
        attachments: payload.attachments,
        source_trace: traceFromEvent(event),
      };
      newEventTraces = trackEvent(nodeId);
      newNodes = [...state.nodes, node];
      newTurnIndex[event.turn_id] = [
        ...(newTurnIndex[event.turn_id] ?? []),
        nodeId,
      ];
      break;
    }

    case "assistant_text_delta": {
      const payload = parsePayload<AssistantTextDeltaPayload>(event);

      // Ignore empty deltas
      if (!payload.delta) {
        newNodes = state.nodes;
        break;
      }

      // Find existing text node with the same node_id
      const existingIdx = state.nodes.findIndex(
        (n) =>
          n.kind === "assistant_text" && n.node_id === payload.node_id,
      );

      if (existingIdx >= 0) {
        const existing = state.nodes[existingIdx] as AssistantTextNode;
        const updated: AssistantTextNode = {
          ...existing,
          content: existing.content + payload.delta,
          byte_length: (existing.byte_length ?? 0) + payload.delta.length,
          updated_at_ms: event.created_at_ms,
          source_trace: mergeTrace(existing.source_trace, event),
        };
        newNodes = [...state.nodes];
        newNodes[existingIdx] = updated;
      } else {
        const node: AssistantTextNode = {
          kind: "assistant_text",
          node_id: payload.node_id,
          turn_id: event.turn_id,
          status: "pending",
          created_at_ms: event.created_at_ms,
          updated_at_ms: event.created_at_ms,
          content: payload.delta,
          byte_length: payload.delta.length,
          source_trace: traceFromEvent(event),
        };
        newNodes = [...state.nodes, node];
        newTurnIndex[event.turn_id] = [
          ...(newTurnIndex[event.turn_id] ?? []),
          payload.node_id,
        ];
      }
      newEventTraces = trackEvent(payload.node_id);
      break;
    }

    case "assistant_text_snapshot": {
      const payload = parsePayload<AssistantTextSnapshotPayload>(event);
      const existingIdx = state.nodes.findIndex(
        (n) =>
          n.kind === "assistant_text" && n.node_id === payload.node_id,
      );

      if (existingIdx >= 0) {
        const existing = state.nodes[existingIdx] as AssistantTextNode;
        const updated: AssistantTextNode = {
          ...existing,
          content: payload.content,
          byte_length: payload.byte_length ?? payload.content.length,
          status: completeStatus(existing.status),
          updated_at_ms: event.created_at_ms,
          source_trace: mergeTrace(existing.source_trace, event),
        };
        newNodes = [...state.nodes];
        newNodes[existingIdx] = updated;
      } else {
        const node: AssistantTextNode = {
          kind: "assistant_text",
          node_id: payload.node_id,
          turn_id: event.turn_id,
          status: "completed",
          created_at_ms: event.created_at_ms,
          updated_at_ms: event.created_at_ms,
          content: payload.content,
          byte_length: payload.byte_length ?? payload.content.length,
          source_trace: traceFromEvent(event),
        };
        newNodes = [...state.nodes, node];
        newTurnIndex[event.turn_id] = [
          ...(newTurnIndex[event.turn_id] ?? []),
          payload.node_id,
        ];
      }
      newEventTraces = trackEvent(payload.node_id);
      break;
    }

    case "reasoning_delta": {
      const payload = parsePayload<ReasoningDeltaPayload>(event);

      // Ignore empty deltas
      if (!payload.delta) {
        newNodes = state.nodes;
        break;
      }

      const existingIdx = state.nodes.findIndex(
        (n) => n.kind === "reasoning" && n.node_id === payload.node_id,
      );

      if (existingIdx >= 0) {
        const existing = state.nodes[existingIdx] as ReasoningNode;
        const updated: ReasoningNode = {
          ...existing,
          content: existing.content + payload.delta,
          updated_at_ms: event.created_at_ms,
          source_trace: mergeTrace(existing.source_trace, event),
        };
        newNodes = [...state.nodes];
        newNodes[existingIdx] = updated;
      } else {
        const node: ReasoningNode = {
          kind: "reasoning",
          node_id: payload.node_id,
          turn_id: event.turn_id,
          status: "pending",
          created_at_ms: event.created_at_ms,
          updated_at_ms: event.created_at_ms,
          content: payload.delta,
          collapsed: false,
          source_trace: traceFromEvent(event),
        };
        newNodes = [...state.nodes, node];
        newTurnIndex[event.turn_id] = [
          ...(newTurnIndex[event.turn_id] ?? []),
          payload.node_id,
        ];
      }
      newEventTraces = trackEvent(payload.node_id);
      break;
    }

    case "reasoning_snapshot": {
      const payload = parsePayload<ReasoningSnapshotPayload>(event);
      const existingIdx = state.nodes.findIndex(
        (n) => n.kind === "reasoning" && n.node_id === payload.node_id,
      );

      if (existingIdx >= 0) {
        const existing = state.nodes[existingIdx] as ReasoningNode;
        const updated: ReasoningNode = {
          ...existing,
          content: payload.content,
          status: completeStatus(existing.status),
          collapsed: true, // collapse after completion
          updated_at_ms: event.created_at_ms,
          source_trace: mergeTrace(existing.source_trace, event),
        };
        newNodes = [...state.nodes];
        newNodes[existingIdx] = updated;
      } else {
        const node: ReasoningNode = {
          kind: "reasoning",
          node_id: payload.node_id,
          turn_id: event.turn_id,
          status: "completed",
          created_at_ms: event.created_at_ms,
          updated_at_ms: event.created_at_ms,
          content: payload.content,
          collapsed: true,
          source_trace: traceFromEvent(event),
        };
        newNodes = [...state.nodes, node];
        newTurnIndex[event.turn_id] = [
          ...(newTurnIndex[event.turn_id] ?? []),
          payload.node_id,
        ];
      }
      newEventTraces = trackEvent(payload.node_id);
      break;
    }

    case "tool_call_started": {
      const payload = parsePayload<ToolCallStartedPayload>(event);
      const nodeId = `node-ts-${payload.call_id}`;
      const node: ToolStepNode = {
        kind: "tool_step",
        node_id: nodeId,
        turn_id: event.turn_id,
        status: "running",
        created_at_ms: event.created_at_ms,
        updated_at_ms: event.created_at_ms,
        tool_name: payload.tool_name,
        tool_category: payload.tool_category as ToolStepNode["tool_category"],
        display_title: payload.display_title ?? payload.tool_name,
        call_id: payload.call_id,
        target: payload.target,
        progress_label: undefined,
        progress: undefined,
        started_at_ms: event.created_at_ms,
        finished_at_ms: undefined,
        duration_ms: undefined,
        output_preview: undefined,
        output_detail: undefined,
        error_message: undefined,
        args: payload.args,
        source_trace: traceFromEvent(event),
      };
      newEventTraces = trackEvent(nodeId);
      newNodes = [...state.nodes, node];
      newTurnIndex[event.turn_id] = [
        ...(newTurnIndex[event.turn_id] ?? []),
        nodeId,
      ];
      break;
    }

    case "tool_call_progress": {
      const payload = parsePayload<ToolCallProgressPayload>(event);
      const nodeId = `node-ts-${payload.call_id}`;
      const existingIdx = state.nodes.findIndex(
        (n) => n.kind === "tool_step" && n.call_id === payload.call_id,
      );

      if (existingIdx >= 0) {
        const existing = state.nodes[existingIdx] as ToolStepNode;
        const updated: ToolStepNode = {
          ...existing,
          status: "running",
          progress_label: payload.message || existing.progress_label,
          progress: payload.progress ?? existing.progress,
          updated_at_ms: event.created_at_ms,
          source_trace: mergeTrace(existing.source_trace, event),
        };
        newNodes = [...state.nodes];
        newNodes[existingIdx] = updated;
      } else {
        // Progress before start — create a stub node
        const node: ToolStepNode = {
          kind: "tool_step",
          node_id: nodeId,
          turn_id: event.turn_id,
          status: "running",
          created_at_ms: event.created_at_ms,
          updated_at_ms: event.created_at_ms,
          tool_name: payload.call_id,
          tool_category: "other",
          display_title: payload.message,
          call_id: payload.call_id,
          progress_label: payload.message,
          progress: payload.progress,
          source_trace: traceFromEvent(event),
        };
        newNodes = [...state.nodes, node];
        newTurnIndex[event.turn_id] = [
          ...(newTurnIndex[event.turn_id] ?? []),
          nodeId,
        ];
      }
      newEventTraces = trackEvent(nodeId);
      break;
    }

    case "tool_call_finished": {
      const payload = parsePayload<ToolCallFinishedPayload>(event);
      const nodeId = `node-ts-${payload.call_id}`;
      const existingIdx = state.nodes.findIndex(
        (n) => n.kind === "tool_step" && n.call_id === payload.call_id,
      );

      if (existingIdx >= 0) {
        const existing = state.nodes[existingIdx] as ToolStepNode;
        const updated: ToolStepNode = {
          ...existing,
          status: successStatus(payload.success),
          tool_name: payload.tool_name || existing.tool_name,
          finished_at_ms: event.created_at_ms,
          duration_ms: payload.duration_ms ?? existing.duration_ms,
          output_preview: payload.output_preview ?? existing.output_preview,
          output_detail: payload.output_detail ?? existing.output_detail,
          error_message: payload.error_message ?? existing.error_message,
          updated_at_ms: event.created_at_ms,
          source_trace: mergeTrace(existing.source_trace, event),
        };
        newNodes = [...state.nodes];
        newNodes[existingIdx] = updated;
      } else {
        // Finished without start — create a completed node
        const node: ToolStepNode = {
          kind: "tool_step",
          node_id: nodeId,
          turn_id: event.turn_id,
          status: successStatus(payload.success),
          created_at_ms: event.created_at_ms,
          updated_at_ms: event.created_at_ms,
          tool_name: payload.tool_name,
          display_title: payload.tool_name,
          call_id: payload.call_id,
          duration_ms: payload.duration_ms,
          output_preview: payload.output_preview,
          output_detail: payload.output_detail,
          error_message: payload.error_message,
          source_trace: traceFromEvent(event),
        };
        newNodes = [...state.nodes, node];
        newTurnIndex[event.turn_id] = [
          ...(newTurnIndex[event.turn_id] ?? []),
          nodeId,
        ];
      }
      newEventTraces = trackEvent(nodeId);
      break;
    }

    case "approval_requested": {
      const payload = parsePayload<ApprovalRequestedPayload>(event);
      const nodeId = `node-ap-${payload.approval_id}`;
      const node: ApprovalNode = {
        kind: "approval",
        node_id: nodeId,
        turn_id: event.turn_id,
        status: "pending",
        created_at_ms: event.created_at_ms,
        updated_at_ms: event.created_at_ms,
        approval_id: payload.approval_id,
        action: payload.action,
        reason: payload.reason,
        risk_level: payload.risk_level,
        source_trace: traceFromEvent(event),
      };
      newEventTraces = trackEvent(nodeId);
      newNodes = [...state.nodes, node];
      newTurnIndex[event.turn_id] = [
        ...(newTurnIndex[event.turn_id] ?? []),
        nodeId,
      ];
      break;
    }

    case "approval_resolved": {
      const payload = parsePayload<ApprovalResolvedPayload>(event);
      const nodeId = `node-ap-${payload.approval_id}`;
      const existingIdx = state.nodes.findIndex(
        (n) => n.kind === "approval" && n.approval_id === payload.approval_id,
      );

      if (existingIdx >= 0) {
        const existing = state.nodes[existingIdx] as ApprovalNode;
        const updated: ApprovalNode = {
          ...existing,
          status: "completed",
          decision: payload.decision,
          decision_source: payload.source,
          updated_at_ms: event.created_at_ms,
          source_trace: mergeTrace(existing.source_trace, event),
        };
        newNodes = [...state.nodes];
        newNodes[existingIdx] = updated;
      } else {
        // Resolved without request — create a completed node
        const node: ApprovalNode = {
          kind: "approval",
          node_id: nodeId,
          turn_id: event.turn_id,
          status: "completed",
          created_at_ms: event.created_at_ms,
          updated_at_ms: event.created_at_ms,
          approval_id: payload.approval_id,
          action: "",
          reason: "",
          decision: payload.decision,
          decision_source: payload.source,
          source_trace: traceFromEvent(event),
        };
        newNodes = [...state.nodes, node];
        newTurnIndex[event.turn_id] = [
          ...(newTurnIndex[event.turn_id] ?? []),
          nodeId,
        ];
      }
      newEventTraces = trackEvent(nodeId);
      break;
    }

    case "iteration_boundary": {
      const payload = parsePayload<IterationBoundaryPayload>(event);
      const nodeId = `node-ib-${payload.iteration}`;
      const node: IterationBoundaryNode = {
        kind: "iteration_boundary",
        node_id: nodeId,
        turn_id: event.turn_id,
        status: "completed",
        created_at_ms: event.created_at_ms,
        updated_at_ms: event.created_at_ms,
        iteration: payload.iteration,
        source_trace: traceFromEvent(event),
      };
      newEventTraces = trackEvent(nodeId);
      newNodes = [...state.nodes, node];
      newTurnIndex[event.turn_id] = [
        ...(newTurnIndex[event.turn_id] ?? []),
        nodeId,
      ];
      break;
    }

    case "assistant_message_finalized": {
      const payload = parsePayload<AssistantMessageFinalizedPayload>(event);
      if (payload.text_node_id) {
        const existingIdx = state.nodes.findIndex(
          (n) =>
            n.kind === "assistant_text" && n.node_id === payload.text_node_id,
        );
        if (existingIdx >= 0) {
          const existing = state.nodes[existingIdx] as AssistantTextNode;
          const updated: AssistantTextNode = {
            ...existing,
            content: payload.final_text_content ?? existing.content,
            status: "completed",
            updated_at_ms: event.created_at_ms,
            source_trace: mergeTrace(existing.source_trace, event),
          };
          newNodes = [...state.nodes];
          newNodes[existingIdx] = updated;
          newEventTraces = trackEvent(payload.text_node_id);
        } else {
          newNodes = state.nodes;
        }
      } else {
        newNodes = state.nodes;
      }
      break;
    }

    case "turn_finished": {
      const payload = parsePayload<TurnFinishedPayload>(event);
      const nodeId = `node-tstatus-${event.turn_id}`;

      // Mark any pending/running nodes in this turn as completed/failed
      const updatedNodes = state.nodes.map((n) => {
        if (n.turn_id !== event.turn_id) return n;
        if (n.kind === "assistant_text" && n.status !== "completed") {
          return { ...n, status: "completed" as NodeStatus, updated_at_ms: event.created_at_ms };
        }
        if (n.kind === "reasoning" && n.status !== "completed") {
          return { ...n, status: "completed" as NodeStatus, collapsed: true, updated_at_ms: event.created_at_ms };
        }
        return n;
      });

      const node: TurnStatusNode = {
        kind: "turn_status",
        node_id: nodeId,
        turn_id: event.turn_id,
        status: payload.severity === "error" ? "failed" : "completed",
        created_at_ms: event.created_at_ms,
        updated_at_ms: event.created_at_ms,
        end_reason: payload.end_reason,
        summary: payload.user_message,
        diagnosis: payload.diagnosis_code
          ? {
              diagnosis_code: payload.diagnosis_code,
              severity: payload.severity,
              user_message: payload.user_message,
              iterations: payload.iterations,
              tool_calls: payload.tool_calls,
            }
          : undefined,
        elapsed_ms: payload.elapsed_ms,
        source_trace: traceFromEvent(event),
      };
      newEventTraces = trackEvent(nodeId);
      newNodes = [...updatedNodes, node];
      newTurnIndex[event.turn_id] = [
        ...(newTurnIndex[event.turn_id] ?? []),
        nodeId,
      ];
      break;
    }

    case "compact_boundary": {
      const payload = parsePayload<CompactBoundaryPayload>(event);
      const nodeId = nextNodeId("cb");
      const node: SystemNoticeNode = {
        kind: "system_notice",
        node_id: nodeId,
        turn_id: event.turn_id,
        status: "completed",
        created_at_ms: event.created_at_ms,
        updated_at_ms: event.created_at_ms,
        message: `Context compacted (${payload.trigger}): ${payload.messages_removed} messages removed`,
        level: "info",
        category: "compaction",
        source_trace: traceFromEvent(event),
      };
      newEventTraces = trackEvent(nodeId);
      newNodes = [...state.nodes, node];
      break;
    }

    case "system_notice": {
      const payload = parsePayload<SystemNoticePayload>(event);
      const nodeId = nextNodeId("sn");
      const node: SystemNoticeNode = {
        kind: "system_notice",
        node_id: nodeId,
        turn_id: event.turn_id,
        status: "completed",
        created_at_ms: event.created_at_ms,
        updated_at_ms: event.created_at_ms,
        message: payload.message,
        level: payload.level,
        category: payload.category,
        source_trace: traceFromEvent(event),
      };
      newEventTraces = trackEvent(nodeId);
      newNodes = [...state.nodes, node];
      break;
    }

    default:
      newNodes = state.nodes;
      break;
  }

  return {
    events: newEvents,
    nodes: newNodes,
    maxSeq: newMaxSeq,
    sessionId: state.sessionId || event.session_id,
    turnIndex: newTurnIndex,
    eventTraces: newEventTraces,
  };
}

/**
 * Replay: reduce timeline events and return only the materialized display nodes.
 */
export function materializeNodes(
  events: TurnTimelineEvent[],
): TurnDisplayNode[] {
  const state = reduceTimelineEvents(events);
  return state.nodes;
}
