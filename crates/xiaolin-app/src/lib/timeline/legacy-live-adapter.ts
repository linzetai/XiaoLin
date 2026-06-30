// Legacy live WebSocket adapter: convert old ChatStreamEvent → synthetic TurnTimelineEvent[].
//
// Legacy events are mapped with stable IDs. Reasoning is never exposed
// (undefined visibility = safe default). Old events NEVER merge with
// authoritative timeline — authoritative snapshot replaces legacy state.

import type { TurnTimelineEvent, TimelineEventType } from "./types";
import { TIMELINE_SCHEMA_VERSION } from "./types";

// ============================================================================
// Types
// ============================================================================

export interface ChatStreamEvent {
  type: string;
  data?: Record<string, unknown>;
  error?: { message?: string };
}

export interface LegacyLiveAdapter {
  ingest(event: ChatStreamEvent): TurnTimelineEvent[];
  flush(): TurnTimelineEvent[];
  createUserMessage(content: string, attachments?: string[]): TurnTimelineEvent[];
}

// ============================================================================
// Factory
// ============================================================================

export function createLegacyLiveAdapter(
  sessionId: string,
): LegacyLiveAdapter {
  let _seq = 0;
  let _ts = Date.now();
  let currentTurnId: string | null = null;
  let textBuffer = "";
  let reasoningBuffer = "";
  let textNodeId = "";
  let currentToolCallId = "";
  let turnStarted = false;

  function nextSeq(): number {
    _seq += 1;
    return _seq;
  }

  function nowMs(): number {
    _ts += 10;
    return _ts;
  }

  function makeEvent(
    turnId: string,
    eventType: TimelineEventType,
    payload: Record<string, unknown>,
    eventId: string,
  ): TurnTimelineEvent {
    return {
      id: eventId,
      session_id: sessionId,
      turn_id: turnId,
      seq: nextSeq(),
      event_type: eventType,
      schema_version: TIMELINE_SCHEMA_VERSION,
      payload_json: payload,
      created_at_ms: nowMs(),
    };
  }

  function flushText(turnId: string): TurnTimelineEvent[] {
    if (!textBuffer) return [];
    const nodeId = textNodeId || `legacy-text-${turnId}`;
    const events: TurnTimelineEvent[] = [
      makeEvent(turnId, "assistant_text_snapshot", {
        node_id: nodeId,
        content: textBuffer,
        text_role: "final",
      }, `legacy:${sessionId}:${turnId}:text-snapshot`),
    ];
    textBuffer = "";
    return events;
  }

  function flushReasoning(_turnId: string): TurnTimelineEvent[] {
    // Legacy reasoning: never expose (undefined visibility = safe default)
    reasoningBuffer = "";
    return [];
  }

  const adapter: LegacyLiveAdapter = {
    ingest(event: ChatStreamEvent): TurnTimelineEvent[] {
      const turnId = currentTurnId || `legacy-turn-${sessionId}-${Date.now()}`;

      switch (event.type) {
        case "turn_start": {
          currentTurnId = event.data?.session_id as string || turnId;
          turnStarted = true;
          textBuffer = "";
          reasoningBuffer = "";
          textNodeId = `legacy-text-${currentTurnId}`;
          return [makeEvent(currentTurnId, "turn_started", {
            session_id: sessionId,
          }, `legacy:${sessionId}:${currentTurnId}:turn_started`)];
        }

        case "content_delta": {
          const delta = (event.data as any)?.choices?.[0]?.delta?.content as string
            || event.data?.delta as string
            || "";
          if (delta) textBuffer += delta;
          return [];
        }

        case "reasoning_delta": {
          // Absorb but never emit (legacy reasoning is hidden)
          const content = (event.data?.content as string) || "";
          if (content) reasoningBuffer += content;
          return [];
        }

        case "tool_executing": {
          const d = event.data || {};
          const callId = (d.call_id as string) || (d.tool_name as string) || "unknown";
          currentToolCallId = callId;
          const flushed = flushText(turnId);
          return [
            ...flushed,
            makeEvent(turnId, "tool_call_started", {
              call_id: callId,
              tool_name: d.tool_name,
              display_title: d.tool_name,
              args: d.args,
            }, `legacy:${sessionId}:${callId}:started`),
          ];
        }

        case "tool_progress": {
          const d = event.data || {};
          const callId = (d.call_id as string) || currentToolCallId;
          return [makeEvent(turnId, "tool_call_progress", {
            call_id: callId,
            message: (d.message as string) || "",
            progress: d.progress as number | undefined,
          }, `legacy:${sessionId}:${callId}:progress-${Date.now()}`)];
        }

        case "tool_result": {
          const d = event.data || {};
          const callId = (d.call_id as string) || currentToolCallId;
          const flushed = flushText(turnId);
          return [
            ...flushed,
            makeEvent(turnId, "tool_call_finished", {
              call_id: callId,
              tool_name: d.tool_name,
              success: d.success ?? true,
              duration_ms: d.duration as number | undefined,
              error_message: d.success === false ? "Tool execution failed" : undefined,
            }, `legacy:${sessionId}:${callId}:finished`),
          ];
        }

        case "approval_required": {
          const d = event.data || {};
          const approvalId = (d.approval_id as string) || `legacy-apr-${Date.now()}`;
          return [makeEvent(turnId, "approval_requested", {
            approval_id: approvalId,
            action: (d.action_type as string) || "unknown",
            reason: (d.reason as string) || "",
            risk_level: d.risk_level,
          }, `legacy:${sessionId}:${approvalId}:requested`)];
        }

        case "approval_resolved": {
          const d = event.data || {};
          const approvalId = (d.approval_id as string) || "";
          return [makeEvent(turnId, "approval_resolved", {
            approval_id: approvalId,
            decision: (d.decision as string) || "allow_once",
            source: "user",
          }, `legacy:${sessionId}:${approvalId}:resolved`)];
        }

        case "turn_end": {
          const flushed = flushText(turnId);
          return [
            ...flushed,
            makeEvent(turnId, "turn_finished", {
              end_reason: "completed",
            }, `legacy:${sessionId}:${turnId}:turn_finished`),
          ];
        }

        case "error": {
          const message = (event.data?.message as string)
            || event.error?.message
            || "Unknown error";
          return [makeEvent(turnId, "system_notice", {
            message,
            level: "error",
            category: "stream_error",
          }, `legacy:${sessionId}:${turnId}:error-${Date.now()}`)];
        }

        default:
          return [];
      }
    },

    flush(): TurnTimelineEvent[] {
      if (!currentTurnId) return [];
      const flushed = flushText(currentTurnId);
      const reasoned = flushReasoning(currentTurnId);
      if (turnStarted) {
        return [
          ...reasoned,
          ...flushed,
          makeEvent(currentTurnId, "turn_finished", {
            end_reason: "completed",
          }, `legacy:${sessionId}:${currentTurnId}:turn_finished-flush`),
        ];
      }
      return [...reasoned, ...flushed];
    },

    createUserMessage(content: string, attachments?: string[]): TurnTimelineEvent[] {
      const turnId = `legacy-turn-${sessionId}-${Date.now()}`;
      currentTurnId = turnId;
      turnStarted = true;
      textBuffer = "";
      reasoningBuffer = "";
      textNodeId = `legacy-text-${turnId}`;
      return [
        makeEvent(turnId, "turn_started", {
          session_id: sessionId,
        }, `legacy:${sessionId}:${turnId}:turn_started`),
        makeEvent(turnId, "user_message_created", {
          content,
          attachments,
        }, `legacy:${sessionId}:${turnId}:user`),
      ];
    },
  };

  return adapter;
}
