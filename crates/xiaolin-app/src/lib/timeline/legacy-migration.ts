// Legacy history migration: convert old ChatMessage[] to canonical TurnTimelineEvent[].
//
// Principles:
// - Consecutive user → assistant(s) → next user = one turn
// - Stable IDs: legacy:{sessionId}:{sourceMessageId}:{segmentIndex}
// - Stable order key: stableSourceMessageOrder * 10_000 + segmentIndex
// - Legacy reasoning is never included
// - Missing segmentOrder: preserve tool summary + final answer + notice

import type { TurnTimelineEvent, TimelineEventType } from "./types";
import { TIMELINE_SCHEMA_VERSION } from "./types";

// ============================================================================
// Types
// ============================================================================

export interface LegacyMessage {
  id: string | number;
  role: "user" | "assistant" | "system";
  content: string;
  timestamp?: Date | string;
  toolCalls?: LegacyToolCall[];
  reasoningContent?: string;
  backendId?: string;
}

export interface LegacyToolCall {
  id: string;
  name: string;
  status: "running" | "success" | "error";
  args?: string;
  result?: string;
  duration?: number;
}

// ============================================================================
// Migration
// ============================================================================

let _seq = 0;
let _ts = 0;

function nextSeq(): number {
  _seq += 1;
  return _seq;
}

function nextCreatedAtMs(timestamp?: Date | string): number {
  if (timestamp) {
    return new Date(timestamp).getTime();
  }
  _ts += 1000;
  return _ts;
}

function makeEvent(
  sessionId: string,
  turnId: string,
  eventType: TimelineEventType,
  payload: Record<string, unknown>,
  createdAtMs: number,
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
    created_at_ms: createdAtMs,
  };
}

export function migrateLegacySessionToTimeline(
  sessionId: string,
  messages: LegacyMessage[],
): TurnTimelineEvent[] {
  // Reset internal counters
  _seq = 0;
  _ts = 0;

  if (messages.length === 0) return [];

  const events: TurnTimelineEvent[] = [];
  let currentTurnId: string | null = null;
  let turnMessages: LegacyMessage[] = [];

  // Assign stable source message order from DB id or creation time
  const stableOrder = new Map<string | number, number>();
  messages.forEach((msg, idx) => {
    stableOrder.set(msg.id, idx + 1);
  });

  const flushTurn = () => {
    if (turnMessages.length === 0) return;
    const turnId = currentTurnId!;
    const userMsg = turnMessages.find((m) => m.role === "user");
    const assistantMsgs = turnMessages.filter((m) => m.role === "assistant");
    const createdAtMs = nextCreatedAtMs(userMsg?.timestamp);

    // turn_started
    events.push(makeEvent(
      sessionId, turnId, "turn_started",
      { session_id: sessionId },
      createdAtMs,
      `legacy:${sessionId}:${turnId}:turn_started`,
    ));

    // user_message_created
    if (userMsg) {
      events.push(makeEvent(
        sessionId, turnId, "user_message_created",
        {
          content: userMsg.content,
          message_id: String(userMsg.id),
        },
        createdAtMs,
        `legacy:${sessionId}:${String(userMsg.id)}:user`,
      ));
    }

    // Process assistant messages in order
    let segmentIndex = 0;
    for (const msg of assistantMsgs) {
      const msgCreatedAt = nextCreatedAtMs(msg.timestamp);

      // Tool calls
      if (msg.toolCalls && msg.toolCalls.length > 0) {
        for (const tc of msg.toolCalls) {
          const callId = tc.id || `legacy-tc-${msg.id}-${segmentIndex}`;
          events.push(makeEvent(
            sessionId, turnId, "tool_call_started",
            {
              call_id: callId,
              tool_name: tc.name,
              display_title: tc.name,
              args: tc.args,
            },
            msgCreatedAt,
            `legacy:${sessionId}:${String(msg.id)}:${segmentIndex++}`,
          ));

          events.push(makeEvent(
            sessionId, turnId, "tool_call_finished",
            {
              call_id: callId,
              tool_name: tc.name,
              success: tc.status === "success",
              duration_ms: tc.duration,
              error_message: tc.status === "error" ? "Tool execution failed" : undefined,
            },
            msgCreatedAt + 1,
            `legacy:${sessionId}:${String(msg.id)}:${segmentIndex++}`,
          ));
        }
      }

      // Final assistant text
      if (msg.content) {
        const nodeId = `legacy-text-${msg.id}`;
        events.push(makeEvent(
          sessionId, turnId, "assistant_text_snapshot",
          {
            node_id: nodeId,
            content: msg.content,
            text_role: "final",
          },
          msgCreatedAt + 2,
          `legacy:${sessionId}:${String(msg.id)}:${segmentIndex++}`,
        ));
      }
    }

    // system_notice: legacy format marker
    events.push(makeEvent(
      sessionId, turnId, "system_notice",
      {
        message: "旧格式会话 — 步骤顺序可能不准确",
        level: "info",
        category: "legacy",
      },
      nextCreatedAtMs(),
      `legacy:${sessionId}:${turnId}:notice`,
    ));

    // turn_finished
    events.push(makeEvent(
      sessionId, turnId, "turn_finished",
      {
        end_reason: "completed",
      },
      nextCreatedAtMs(),
      `legacy:${sessionId}:${turnId}:turn_finished`,
    ));
  };

  for (const msg of messages) {
    if (msg.role === "user" && currentTurnId != null) {
      // User message starts a new turn — flush the current one
      flushTurn();
      turnMessages = [];
    }

    if (currentTurnId == null) {
      currentTurnId = `legacy-turn-${msg.id}`;
    }

    turnMessages.push(msg);
  }

  // Flush last turn
  flushTurn();

  return events;
}
