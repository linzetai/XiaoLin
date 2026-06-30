// TurnBlock — renders a single turn as a user-message row followed by an
// assistant response block (Codex app / ChatGPT-style message grouping).
//
// Each turn is the primary visual unit: the user's message is an inline
// transcript row, and the assistant's response (text, reasoning, tool activity,
// etc.) is rendered as a cohesive left-aligned block.

import { memo } from "react";
import type { TurnGroup } from "../../lib/timeline/selectors";
import type { UserMessageNode } from "../../lib/timeline/types";
import type { ChatMessage } from "../../lib/stores/types";
import { AssistantResponseBlock } from "./AssistantResponseBlock";
import { PhaseIndicator } from "./ThinkingIndicator";
import { UserInput } from "./UserInput";

export interface TurnBlockProps {
  turnGroup: TurnGroup;
  /** When true, pending nodes show streaming animations. */
  isLive?: boolean;
  /** Session ID for sub-components that need it (tool output details). */
  sessionId?: string;
  /** When true, diagnostic-only timeline metadata can be visible. */
  showDiagnostics?: boolean;
}

/**
 * Single turn block: user message → assistant response.
 *
 * Handles edge cases:
 * - System-initiated turns (no user message) — skips the user bubble.
 * - Pending turns (user message only, no assistant nodes yet) — shows just the
 *   user row while the assistant response is being generated.
 */
export const TurnBlock = memo(function TurnBlock({
  turnGroup,
  isLive,
  sessionId,
  showDiagnostics,
}: TurnBlockProps) {
  const { userMessageNode, assistantNodes } = turnGroup;

  return (
    <div className="turn-block mb-6 min-w-0 w-full max-w-full">
      {/* User message — Codex App style right-aligned prompt bubble */}
      {userMessageNode && (
        <UserMessageRow node={userMessageNode} />
      )}

      {/* Assistant response — left-aligned cohesive block */}
      {assistantNodes.length > 0 ? (
        <AssistantResponseBlock
          nodes={assistantNodes}
          isLive={isLive}
          sessionId={sessionId}
          showDiagnostics={showDiagnostics}
        />
      ) : isLive ? (
        <PhaseIndicator phase={userMessageNode?.status === "pending" ? "connecting" : "thinking"} />
      ) : null}
    </div>
  );
});

// ============================================================================
// UserMessageRow
// ============================================================================

const UserMessageRow = memo(function UserMessageRow({
  node,
}: {
  node: UserMessageNode;
}) {
  const msg: ChatMessage = {
    role: "user",
    content: node.content,
    id: 0,
    timestamp: new Date(node.created_at_ms),
    chatId: node.turn_id,
  };

  return <UserInput msg={msg} copyable />;
});
