// Timeline test fixtures.
//
// Factory functions for building TurnTimelineEvent values in tests.
// Each factory accepts optional overrides so tests can customize just the
// fields they care about.

import type {
  TurnTimelineEvent,
  TimelineEventType,
} from "./types";
import { TIMELINE_SCHEMA_VERSION } from "./types";

// ============================================================================
// Factory helpers
// ============================================================================

let _seq = 0;
let _ts = 1700000000000;

function nextSeq(): number {
  _seq += 1;
  return _seq;
}

function nextId(prefix: string): string {
  return `evt-${prefix}-${nextSeq()}`;
}

function nowMs(): number {
  _ts += 1000;
  return _ts;
}

function makeEvent(
  type: TimelineEventType,
  overrides: Partial<TurnTimelineEvent> & { payload?: Record<string, unknown> },
): TurnTimelineEvent {
  const { payload, ...rest } = overrides;
  return {
    id: nextId(type),
    session_id: "session-test-1",
    turn_id: "turn-test-1",
    seq: nextSeq(),
    event_type: type,
    schema_version: TIMELINE_SCHEMA_VERSION,
    payload_json: payload ?? {},
    created_at_ms: nowMs(),
    ...rest,
  };
}

// ============================================================================
// Event factories
// ============================================================================

export function makeTurnStarted(
  overrides: Partial<TurnTimelineEvent> & {
    payload?: { session_id?: string; execution_mode?: string; agent_id?: string };
  } = {},
): TurnTimelineEvent {
  const { payload, ...rest } = overrides;
  return makeEvent("turn_started", {
    payload: {
      session_id: "session-test-1",
      execution_mode: "agent",
      agent_id: "main",
      ...payload,
    },
    ...rest,
  });
}

export function makeUserMessageCreated(
  overrides: Partial<TurnTimelineEvent> & {
    payload?: { content?: string; message_id?: string; attachments?: string[] };
  } = {},
): TurnTimelineEvent {
  const { payload, ...rest } = overrides;
  return makeEvent("user_message_created", {
    payload: {
      content: "Hello, world!",
      ...payload,
    },
    ...rest,
  });
}

export function makeTextDelta(
  overrides: Partial<TurnTimelineEvent> & {
    payload?: { node_id?: string; delta?: string; offset?: number };
  } = {},
): TurnTimelineEvent {
  const { payload, ...rest } = overrides;
  return makeEvent("assistant_text_delta", {
    payload: {
      node_id: "node-at-1",
      delta: "Hello ",
      offset: 0,
      ...payload,
    },
    ...rest,
  });
}

export function makeTextSnapshot(
  overrides: Partial<TurnTimelineEvent> & {
    payload?: { node_id?: string; content?: string; byte_length?: number };
  } = {},
): TurnTimelineEvent {
  const { payload, ...rest } = overrides;
  return makeEvent("assistant_text_snapshot", {
    payload: {
      node_id: "node-at-1",
      content: "Hello, world!",
      byte_length: 13,
      ...payload,
    },
    ...rest,
  });
}

export function makeReasoningDelta(
  overrides: Partial<TurnTimelineEvent> & {
    payload?: { node_id?: string; delta?: string; offset?: number };
  } = {},
): TurnTimelineEvent {
  const { payload, ...rest } = overrides;
  return makeEvent("reasoning_delta", {
    payload: {
      node_id: "node-r-1",
      delta: "Let me think...",
      offset: 0,
      ...payload,
    },
    ...rest,
  });
}

export function makeReasoningSnapshot(
  overrides: Partial<TurnTimelineEvent> & {
    payload?: { node_id?: string; content?: string };
  } = {},
): TurnTimelineEvent {
  const { payload, ...rest } = overrides;
  return makeEvent("reasoning_snapshot", {
    payload: {
      node_id: "node-r-1",
      content: "I need to read the file first.",
      ...payload,
    },
    ...rest,
  });
}

export function makeToolStarted(
  overrides: Partial<TurnTimelineEvent> & {
    payload?: {
      call_id?: string;
      tool_name?: string;
      tool_category?: string;
      display_title?: string;
      args?: string;
    };
  } = {},
): TurnTimelineEvent {
  const { payload, ...rest } = overrides;
  return makeEvent("tool_call_started", {
    payload: {
      call_id: "tc-1",
      tool_name: "read_file",
      tool_category: "file",
      display_title: "Read src/main.rs",
      args: '{"path":"src/main.rs"}',
      ...payload,
    },
    ...rest,
  });
}

export function makeToolProgress(
  overrides: Partial<TurnTimelineEvent> & {
    payload?: {
      call_id?: string;
      message?: string;
      progress?: number;
      partial_output?: string;
    };
  } = {},
): TurnTimelineEvent {
  const { payload, ...rest } = overrides;
  return makeEvent("tool_call_progress", {
    payload: {
      call_id: "tc-1",
      message: "Reading file...",
      progress: 0.5,
      ...payload,
    },
    ...rest,
  });
}

export function makeToolFinished(
  overrides: Partial<TurnTimelineEvent> & {
    payload?: {
      call_id?: string;
      tool_name?: string;
      success?: boolean;
      duration_ms?: number;
      output_preview?: Record<string, unknown>;
      output_detail?: Record<string, unknown>;
      error_message?: string;
    };
  } = {},
): TurnTimelineEvent {
  const { payload, ...rest } = overrides;
  return makeEvent("tool_call_finished", {
    payload: {
      call_id: "tc-1",
      tool_name: "read_file",
      success: true,
      duration_ms: 150,
      output_preview: {
        content: "fn main() {\n    println!(\"Hello\");\n}\n",
        byte_length: 36,
        line_count: 3,
        estimated_tokens: 10,
        is_binary: false,
        content_type: "text",
      },
      ...payload,
    },
    ...rest,
  });
}

export function makeApprovalRequested(
  overrides: Partial<TurnTimelineEvent> & {
    payload?: {
      approval_id?: string;
      action?: string;
      reason?: string;
      risk_level?: string;
    };
  } = {},
): TurnTimelineEvent {
  const { payload, ...rest } = overrides;
  return makeEvent("approval_requested", {
    payload: {
      approval_id: "apr-1",
      action: "execute_command",
      reason: "This command may modify files.",
      risk_level: "medium",
      ...payload,
    },
    ...rest,
  });
}

export function makeApprovalResolved(
  overrides: Partial<TurnTimelineEvent> & {
    payload?: {
      approval_id?: string;
      decision?: string;
      source?: string;
    };
  } = {},
): TurnTimelineEvent {
  const { payload, ...rest } = overrides;
  return makeEvent("approval_resolved", {
    payload: {
      approval_id: "apr-1",
      decision: "allow_once",
      source: "user",
      ...payload,
    },
    ...rest,
  });
}

export function makeIterationBoundary(
  overrides: Partial<TurnTimelineEvent> & {
    payload?: { iteration?: number };
  } = {},
): TurnTimelineEvent {
  const { payload, ...rest } = overrides;
  return makeEvent("iteration_boundary", {
    payload: {
      iteration: 1,
      ...payload,
    },
    ...rest,
  });
}

export function makeAssistantMessageFinalized(
  overrides: Partial<TurnTimelineEvent> & {
    payload?: { text_node_id?: string; final_text_content?: string };
  } = {},
): TurnTimelineEvent {
  const { payload, ...rest } = overrides;
  return makeEvent("assistant_message_finalized", {
    payload: {
      text_node_id: "node-at-1",
      final_text_content: "Hello, world! The answer is 42.",
      ...payload,
    },
    ...rest,
  });
}

export function makeTurnFinished(
  overrides: Partial<TurnTimelineEvent> & {
    payload?: {
      end_reason?: string;
      diagnosis_code?: string;
      severity?: string;
      user_message?: string;
      iterations?: number;
      tool_calls?: number;
      elapsed_ms?: number;
    };
  } = {},
): TurnTimelineEvent {
  const { payload, ...rest } = overrides;
  return makeEvent("turn_finished", {
    payload: {
      end_reason: "completed",
      iterations: 1,
      tool_calls: 3,
      elapsed_ms: 5000,
      ...payload,
    },
    ...rest,
  });
}

export function makeCompactBoundary(
  overrides: Partial<TurnTimelineEvent> & {
    payload?: {
      trigger?: string;
      pre_compact_tokens?: number;
      post_compact_tokens?: number;
      messages_removed?: number;
    };
  } = {},
): TurnTimelineEvent {
  const { payload, ...rest } = overrides;
  return makeEvent("compact_boundary", {
    payload: {
      trigger: "auto",
      pre_compact_tokens: 50000,
      post_compact_tokens: 15000,
      messages_removed: 20,
      ...payload,
    },
    ...rest,
  });
}

export function makeSystemNotice(
  overrides: Partial<TurnTimelineEvent> & {
    payload?: { message?: string; level?: string; category?: string };
  } = {},
): TurnTimelineEvent {
  const { payload, ...rest } = overrides;
  return makeEvent("system_notice", {
    payload: {
      message: "Context was compacted.",
      level: "info",
      category: "compaction",
      ...payload,
    },
    ...rest,
  });
}

// ============================================================================
// Complex fixture: full turn with text, reasoning, tools, approval, boundaries
// ============================================================================

/**
 * A complex turn containing:
 * - turn_started
 * - user_message_created
 * - reasoning_delta x2 → reasoning_snapshot
 * - assistant_text_delta x2
 * - tool_call_started → tool_call_progress → tool_call_finished
 * - another tool_call_started → tool_call_finished (with large output detail)
 * - iteration_boundary
 * - approval_requested → approval_resolved
 * - assistant_text_delta (final)
 * - assistant_message_finalized
 * - turn_finished
 */
export function complexTurnFixture(
  sessionId = "session-cplx-1",
  turnId = "turn-cplx-1",
): TurnTimelineEvent[] {
  const events: TurnTimelineEvent[] = [];

  // Reset global counters for deterministic fixture generation
  _seq = 0;
  _ts = 1700000000000;

  // 1. Turn started
  events.push(
    makeTurnStarted({
      session_id: sessionId,
      turn_id: turnId,
      seq: nextSeq(),
      payload: { session_id: sessionId, execution_mode: "agent", agent_id: "main" },
    }),
  );

  // 2. User message
  events.push(
    makeUserMessageCreated({
      session_id: sessionId,
      turn_id: turnId,
      seq: nextSeq(),
      payload: { content: "Please analyze src/main.rs and fix the bug." },
    }),
  );

  // 3. Reasoning deltas + snapshot
  events.push(
    makeReasoningDelta({
      session_id: sessionId,
      turn_id: turnId,
      seq: nextSeq(),
      payload: { node_id: "node-r-1", delta: "Let me read the file first. " },
    }),
  );
  events.push(
    makeReasoningDelta({
      session_id: sessionId,
      turn_id: turnId,
      seq: nextSeq(),
      payload: { node_id: "node-r-1", delta: "I need to understand the structure." },
    }),
  );
  events.push(
    makeReasoningSnapshot({
      session_id: sessionId,
      turn_id: turnId,
      seq: nextSeq(),
      payload: { node_id: "node-r-1", content: "Let me read the file first. I need to understand the structure." },
    }),
  );

  // 4. Text deltas (before tools)
  events.push(
    makeTextDelta({
      session_id: sessionId,
      turn_id: turnId,
      seq: nextSeq(),
      payload: { node_id: "node-at-1", delta: "I'll start by reading the file. " },
    }),
  );

  // 5. First tool call: read_file
  events.push(
    makeToolStarted({
      session_id: sessionId,
      turn_id: turnId,
      seq: nextSeq(),
      payload: {
        call_id: "tc-read-1",
        tool_name: "read_file",
        tool_category: "file",
        display_title: "Read src/main.rs",
        args: '{"path":"src/main.rs"}',
      },
    }),
  );
  events.push(
    makeToolProgress({
      session_id: sessionId,
      turn_id: turnId,
      seq: nextSeq(),
      payload: { call_id: "tc-read-1", message: "Reading...", progress: 0.5 },
    }),
  );
  events.push(
    makeToolFinished({
      session_id: sessionId,
      turn_id: turnId,
      seq: nextSeq(),
      payload: {
        call_id: "tc-read-1",
        tool_name: "read_file",
        success: true,
        duration_ms: 120,
        output_preview: {
          content: "fn main() {\n    let x = 1;\n    println!(\"{x}\");\n}\n",
          byte_length: 51,
          line_count: 4,
          estimated_tokens: 12,
          is_binary: false,
          content_type: "text",
        },
      },
    }),
  );

  // 6. Intermediate text
  events.push(
    makeTextDelta({
      session_id: sessionId,
      turn_id: turnId,
      seq: nextSeq(),
      payload: { node_id: "node-at-1", delta: "\n\nThe file looks simple. Let me check for bugs with grep." },
    }),
  );

  // 7. Second tool call: grep (with large output detail)
  events.push(
    makeToolStarted({
      session_id: sessionId,
      turn_id: turnId,
      seq: nextSeq(),
      payload: {
        call_id: "tc-grep-2",
        tool_name: "grep",
        tool_category: "search",
        display_title: "Search for errors",
        args: '{"pattern":"error"}',
      },
    }),
  );
  events.push(
    makeToolFinished({
      session_id: sessionId,
      turn_id: turnId,
      seq: nextSeq(),
      payload: {
        call_id: "tc-grep-2",
        tool_name: "grep",
        success: true,
        duration_ms: 350,
        output_preview: undefined as unknown as Record<string, unknown>,
        output_detail: {
          handle: "out_abc12345_def56789",
          byte_length: 50000,
          line_count: 1200,
          is_expandable: true,
          size_class: "large",
          summary: "Found 45 matches across 12 files (50,000 bytes, 1,200 lines)",
          content_type: "search_results",
        },
      },
    }),
  );

  // 8. Iteration boundary
  events.push(
    makeIterationBoundary({
      session_id: sessionId,
      turn_id: turnId,
      seq: nextSeq(),
      payload: { iteration: 1 },
    }),
  );

  // 9. Third tool: shell_exec (needs approval)
  events.push(
    makeToolStarted({
      session_id: sessionId,
      turn_id: turnId,
      seq: nextSeq(),
      payload: {
        call_id: "tc-shell-3",
        tool_name: "shell_exec",
        tool_category: "shell",
        display_title: "Run cargo check",
        args: '{"command":"cargo check"}',
      },
    }),
  );

  // 10. Approval
  events.push(
    makeApprovalRequested({
      session_id: sessionId,
      turn_id: turnId,
      seq: nextSeq(),
      payload: {
        approval_id: "apr-1",
        action: "execute_command",
        reason: "Running cargo check",
        risk_level: "low",
      },
    }),
  );
  events.push(
    makeApprovalResolved({
      session_id: sessionId,
      turn_id: turnId,
      seq: nextSeq(),
      payload: {
        approval_id: "apr-1",
        decision: "allow_once",
        source: "user",
      },
    }),
  );

  // 11. Tool finishes
  events.push(
    makeToolFinished({
      session_id: sessionId,
      turn_id: turnId,
      seq: nextSeq(),
      payload: {
        call_id: "tc-shell-3",
        tool_name: "shell_exec",
        success: true,
        duration_ms: 5000,
        output_preview: {
          content: "    Checking myproject v0.1.0\n    Finished dev [unoptimized] target(s) in 0.15s\n",
          byte_length: 79,
          line_count: 2,
          estimated_tokens: 20,
          is_binary: false,
          content_type: "command_output",
        },
      },
    }),
  );

  // 12. Final text
  events.push(
    makeTextDelta({
      session_id: sessionId,
      turn_id: turnId,
      seq: nextSeq(),
      payload: { node_id: "node-at-1", delta: "\n\nNo bugs found. The code compiles cleanly!" },
    }),
  );

  // 13. Assistant message finalized
  events.push(
    makeAssistantMessageFinalized({
      session_id: sessionId,
      turn_id: turnId,
      seq: nextSeq(),
      payload: {
        text_node_id: "node-at-1",
        final_text_content:
          "I'll start by reading the file. \n\nThe file looks simple. Let me check for bugs with grep.\n\nNo bugs found. The code compiles cleanly!",
      },
    }),
  );

  // 14. Turn finished
  events.push(
    makeTurnFinished({
      session_id: sessionId,
      turn_id: turnId,
      seq: nextSeq(),
      payload: {
        end_reason: "completed",
        iterations: 1,
        tool_calls: 3,
        elapsed_ms: 7000,
      },
    }),
  );

  return events;
}

/**
 * A simple text-only turn (no tools, no reasoning).
 */
export function simpleTextTurnFixture(
  sessionId = "session-simple-1",
  turnId = "turn-simple-1",
): TurnTimelineEvent[] {
  _seq = 0;
  _ts = 1700000000000;

  return [
    makeTurnStarted({
      session_id: sessionId,
      turn_id: turnId,
      seq: nextSeq(),
    }),
    makeUserMessageCreated({
      session_id: sessionId,
      turn_id: turnId,
      seq: nextSeq(),
      payload: { content: "What is 2+2?" },
    }),
    makeTextDelta({
      session_id: sessionId,
      turn_id: turnId,
      seq: nextSeq(),
      payload: { node_id: "node-at-1", delta: "2+2" },
    }),
    makeTextDelta({
      session_id: sessionId,
      turn_id: turnId,
      seq: nextSeq(),
      payload: { node_id: "node-at-1", delta: " = 4" },
    }),
    makeAssistantMessageFinalized({
      session_id: sessionId,
      turn_id: turnId,
      seq: nextSeq(),
      payload: { text_node_id: "node-at-1", final_text_content: "2+2 = 4" },
    }),
    makeTurnFinished({
      session_id: sessionId,
      turn_id: turnId,
      seq: nextSeq(),
      payload: { end_reason: "completed" },
    }),
  ];
}

/**
 * A tool-loop termination fixture.
 */
export function toolLoopTerminationFixture(
  sessionId = "session-loop-1",
  turnId = "turn-loop-1",
): TurnTimelineEvent[] {
  _seq = 0;
  _ts = 1700000000000;

  return [
    makeTurnStarted({
      session_id: sessionId,
      turn_id: turnId,
      seq: nextSeq(),
    }),
    makeUserMessageCreated({
      session_id: sessionId,
      turn_id: turnId,
      seq: nextSeq(),
      payload: { content: "Fix all bugs in the project." },
    }),
    makeTextDelta({
      session_id: sessionId,
      turn_id: turnId,
      seq: nextSeq(),
      payload: { node_id: "node-at-1", delta: "Let me search for bugs." },
    }),
    makeToolStarted({
      session_id: sessionId,
      turn_id: turnId,
      seq: nextSeq(),
      payload: {
        call_id: "tc-1",
        tool_name: "grep",
        tool_category: "search",
        display_title: "Search for TODO",
        args: '{"pattern":"TODO"}',
      },
    }),
    makeToolFinished({
      session_id: sessionId,
      turn_id: turnId,
      seq: nextSeq(),
      payload: {
        call_id: "tc-1",
        tool_name: "grep",
        success: true,
        duration_ms: 100,
        output_preview: {
          content: "src/main.rs:5: // TODO: fix this",
          byte_length: 30,
          line_count: 1,
          estimated_tokens: 8,
          is_binary: false,
          content_type: "search_results",
        },
      },
    }),
    // Turn finished with tool_loop diagnosis — partial text before status
    makeTurnFinished({
      session_id: sessionId,
      turn_id: turnId,
      seq: nextSeq(),
      payload: {
        end_reason: "tool_loop",
        diagnosis_code: "tool_loop",
        severity: "error",
        user_message: "Turn stopped by tool loop protection.",
        iterations: 12,
        tool_calls: 45,
        elapsed_ms: 35000,
      },
    }),
  ];
}
