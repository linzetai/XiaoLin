// TurnNodeRenderer — renders TurnDisplayNode[] from the canonical timeline store.
//
// Each TurnDisplayNode variant gets its own view component. Both live
// streaming and history replay use the same renderer (Decision D3).
//
// This is the canonical transcript renderer for both live and replayed turns.

import { memo, lazy, Suspense, useState } from "react";
import { useTranslation } from "react-i18next";
import type {
  TurnDisplayNode,
  UserMessageNode,
  AssistantTextNode,
  ReasoningNode,
  ToolStepNode,
  ToolGroupNode,
  ApprovalNode,
  IterationBoundaryNode,
  TurnStatusNode,
  SystemNoticeNode,
} from "../../lib/timeline/types";

import { UserInput } from "./UserInput";
import { ReasoningBlock } from "./ReasoningBlock";
import { ToolStepView } from "./ToolStepView";

const MarkdownContent = lazy(() =>
  import("./MarkdownContent").then((m) => ({ default: m.MarkdownContent })),
);
import { StreamingMarkdown } from "./StreamingMarkdown";

// ============================================================================
// UserMessageNodeView
// ============================================================================

const UserMessageNodeView = memo(function UserMessageNodeView({
  node,
}: {
  node: UserMessageNode;
}) {
  const msg = {
    role: "user" as const,
    content: node.content,
    id: 0, // synthetic id for legacy component
    timestamp: new Date(node.created_at_ms),
    chatId: "",
  };
  return <UserInput msg={msg} copyable />;
});

// ============================================================================
// AssistantTextNodeView
// ============================================================================

const AssistantTextNodeView = memo(function AssistantTextNodeView({
  node,
  isStreaming,
}: {
  node: AssistantTextNode;
  /** When true, render as in-progress streaming with cursor */
  isStreaming?: boolean;
}) {
  if (!node.content) return null;

  const isActive = isStreaming && node.status === "pending";

  return (
    <div
      className="min-w-0 w-full max-w-full py-1.5 text-[14px] leading-7"
      style={{ color: "var(--fill-primary)" }}
    >
      {isActive ? (
        <StreamingMarkdown content={node.content} />
      ) : (
        <Suspense
          fallback={
            <div
              className="animate-pulse rounded py-1"
              style={{ background: "var(--bg-tertiary)", height: 16 }}
            />
          }
        >
          <MarkdownContent content={node.content} />
        </Suspense>
      )}
      {isActive && (
        <span
          className="ml-0.5 inline-block h-[16px] w-[2px] translate-y-[3px] rounded-full"
          style={{
            background: "var(--tint)",
            animation: "cursor-blink 1s step-end infinite",
          }}
        />
      )}
    </div>
  );
});

// ============================================================================
// ReasoningNodeView
// ============================================================================

const ReasoningNodeView = memo(function ReasoningNodeView({
  node,
  isStreaming,
}: {
  node: ReasoningNode;
  /** When true in live mode, the node may be actively receiving deltas */
  isStreaming?: boolean;
}) {
  const isActive = isStreaming && node.status === "pending";

  return (
    <ReasoningBlock
      content={node.content}
      isStreaming={isActive}
      autoCollapse={node.collapsed}
    />
  );
});

// ============================================================================
// ToolStepNodeView
// ============================================================================

const ToolStepNodeView = memo(function ToolStepNodeView({
  node,
  sessionId,
}: {
  node: ToolStepNode;
  sessionId?: string;
}) {
  return <ToolStepView node={node} sessionId={sessionId} />;
});

// ============================================================================
// ToolGroupNodeView
// ============================================================================

const ToolGroupNodeView = memo(function ToolGroupNodeView({
  node,
  sessionId,
}: {
  node: ToolGroupNode;
  sessionId?: string;
}) {
  const [expanded, setExpanded] = useState(!node.collapsed);
  if (node.steps.length === 0) return null;

  const failedCount = node.steps.filter((step) => step.status === "failed" || step.status === "cancelled").length;
  const runningCount = node.steps.filter((step) => step.status === "running").length;
  const statusText = runningCount > 0
    ? `${runningCount} running`
    : failedCount > 0
      ? `${failedCount} failed`
      : `${node.step_count} completed`;

  return (
    <div className="my-0.5">
      <button
        type="button"
        onClick={() => setExpanded((value) => !value)}
        className="flex w-full items-center gap-2 rounded px-1.5 py-1 text-left transition-colors duration-100"
        style={{ color: "var(--fill-tertiary)" }}
        aria-expanded={expanded}
      >
        <span
          className="inline-block h-[6px] w-[6px] shrink-0 rounded-full"
          style={{
            background: failedCount > 0
              ? "var(--red)"
              : runningCount > 0
                ? "var(--tint)"
                : "var(--green)",
          }}
        />
        <span className="min-w-0 flex-1 truncate text-[12px] font-medium">
          {node.group_label}
        </span>
        <span className="shrink-0 text-[10px] tabular-nums" style={{ color: "var(--fill-quaternary)" }}>
          {statusText}
        </span>
      </button>
      {expanded && (
        <div className="ml-4 border-l pl-2" style={{ borderColor: "var(--separator)" }}>
          {node.steps.map((step) => (
            <ToolStepView key={step.node_id} node={step} sessionId={sessionId} />
          ))}
        </div>
      )}
    </div>
  );
});

// ============================================================================
// ApprovalNodeView
// ============================================================================

const ApprovalNodeView = memo(function ApprovalNodeView({
  node,
}: {
  node: ApprovalNode;
}) {
  return (
    <div className="my-1.5 flex items-center gap-1.5 text-[12px]" style={{ color: "var(--fill-tertiary)" }}>
      <span className="h-1.5 w-1.5 rounded-full" style={{ background: "var(--amber)" }} />
      <span>
        {node.action}:{" "}
        {node.decision
          ? `Resolved — ${node.decision} (via ${node.decision_source ?? "unknown"})`
          : "Awaiting decision…"}
      </span>
      {node.reason && (
        <span className="ml-1 opacity-60">({node.reason})</span>
      )}
    </div>
  );
});

// ============================================================================
// IterationBoundaryView
// ============================================================================

const IterationBoundaryView = memo(function IterationBoundaryView({
  node,
  showDiagnostics,
}: {
  node: IterationBoundaryNode;
  showDiagnostics?: boolean;
}) {
  if (!showDiagnostics) return null;

  return (
    <div
      className="my-3 flex items-center gap-2 select-none"
      style={{ color: "var(--fill-quaternary)" }}
      aria-hidden="true"
    >
      <span className="h-px flex-1" style={{ background: "var(--separator)" }} />
      <span className="iteration-label shrink-0 text-[10px] font-medium opacity-50">
        Iteration {node.iteration}
      </span>
    </div>
  );
});

// ============================================================================
// TurnStatusView
// ============================================================================

const TurnStatusView = memo(function TurnStatusView({
  node,
}: {
  node: TurnStatusNode;
}) {
  const { t } = useTranslation("chat");

  const isAbnormal =
    node.end_reason === "tool_loop" ||
    node.end_reason === "interrupted" ||
    node.end_reason === "replaced" ||
    node.end_reason === "budget_limited" ||
    node.diagnosis?.severity === "error" ||
    node.status === "failed";

  if (!isAbnormal && node.end_reason === "completed") {
    // Normal completion — minimal display
    return null;
  }

  const message =
    node.summary ??
    node.diagnosis?.user_message ??
    getDefaultTurnEndMessage(node.end_reason, t);

  return (
    <div
      className="my-2 flex items-start gap-2 rounded-lg px-3 py-2 text-[12px]"
      style={{
        background:
          node.status === "failed"
            ? "color-mix(in srgb, var(--red) 6%, transparent)"
            : "color-mix(in srgb, var(--amber) 6%, transparent)",
        border:
          node.status === "failed"
            ? "0.5px solid color-mix(in srgb, var(--red) 20%, transparent)"
            : "0.5px solid color-mix(in srgb, var(--amber) 20%, transparent)",
        color: "var(--fill-secondary)",
      }}
    >
      <span className="mt-[2px] inline-block h-[8px] w-[8px] shrink-0 rounded-full" style={{
        background: node.status === "failed" ? "var(--red)" : "var(--amber)",
      }} />
      <div className="flex-1 min-w-0">
        <span className="font-medium">{message}</span>
        {node.diagnosis?.diagnosis_code && (
          <span className="ml-1 opacity-50">
            [{node.diagnosis.diagnosis_code}]
          </span>
        )}
        {node.elapsed_ms != null && (
          <span className="ml-2 opacity-40 tabular-nums">
            {formatElapsed(node.elapsed_ms)}
          </span>
        )}
      </div>
    </div>
  );
});

function getDefaultTurnEndMessage(
  endReason: string,
  t: ReturnType<typeof useTranslation<"chat">>["t"],
): string {
  switch (endReason) {
    case "tool_loop":
      return t("turnEnded_toolLoop", "Turn stopped: tool loop detected");
    case "interrupted":
      return t("turnEnded_interrupted", "Turn was interrupted");
    case "budget_limited":
      return t("turnEnded_budget", "Token budget reached");
    case "cancelled":
      return t("turnEnded_cancelled", "Turn was cancelled");
    default:
      return t("turnEnded_abnormal", "Turn ended abnormally");
  }
}

// ============================================================================
// SystemNoticeView
// ============================================================================

const SystemNoticeView = memo(function SystemNoticeView({
  node,
}: {
  node: SystemNoticeNode;
}) {
  const isError = node.level === "error";
  const isWarning = node.level === "warning";

  return (
    <div
      className="pb-2 flex items-start gap-2 text-[13px]"
      style={{
        color: isError
          ? "var(--red)"
          : isWarning
            ? "var(--amber)"
            : "var(--fill-tertiary)",
        overflowWrap: "break-word",
      }}
    >
      <span
        className="mt-[7px] inline-block h-[6px] w-[6px] shrink-0 rounded-full"
        style={{
          background: isError
            ? "var(--red)"
            : isWarning
              ? "var(--amber)"
              : "var(--tint)",
        }}
      />
      <span className="break-words min-w-0">{node.message}</span>
    </div>
  );
});

// ============================================================================
// TurnNodeRenderer — main renderer
// ============================================================================

export interface TurnNodeRendererProps {
  /** The display nodes to render, in transcript order. */
  nodes: TurnDisplayNode[];
  /** When true, nodes with "pending" status are rendered as streaming. */
  isLive?: boolean;
  /** Session ID for passing to sub-components that need it. */
  sessionId?: string;
  /** When true, diagnostic-only timeline metadata can be visible. */
  showDiagnostics?: boolean;
}

/**
 * Render an array of TurnDisplayNode using node-specific components.
 *
 * Both live WebSocket streaming and history replay use this same renderer.
 * The `isLive` flag controls whether pending nodes show streaming animations.
 */
export const TurnNodeRenderer = memo(function TurnNodeRenderer({
  nodes,
  isLive,
  sessionId,
  showDiagnostics,
}: TurnNodeRendererProps) {
  if (nodes.length === 0) return null;

  return (
    <>
      {nodes.map((node) => (
        <TurnNodeView
          key={node.node_id}
          node={node}
          isLive={isLive}
          sessionId={sessionId}
          showDiagnostics={showDiagnostics}
        />
      ))}
    </>
  );
});

function TurnNodeView({
  node,
  isLive,
  sessionId,
  showDiagnostics,
}: {
  node: TurnDisplayNode;
  isLive?: boolean;
  sessionId?: string;
  showDiagnostics?: boolean;
}) {
  const isStreaming = isLive && node.status === "pending";
  if (
    node.kind === "turn_status" &&
    node.end_reason === "completed" &&
    node.diagnosis == null &&
    node.status !== "failed"
  ) {
    return null;
  }
  if (node.kind === "iteration_boundary" && !showDiagnostics) {
    return null;
  }

  const isActivity =
    node.kind === "reasoning" ||
    node.kind === "tool_step" ||
    node.kind === "tool_group" ||
    node.kind === "approval" ||
    node.kind === "turn_status" ||
    node.kind === "system_notice";

  const content = (() => {
    switch (node.kind) {
      case "user_message":
        return <UserMessageNodeView node={node} />;
      case "assistant_text":
        return (
          <AssistantTextNodeView
            node={node}
            isStreaming={isStreaming}
          />
        );
      case "reasoning":
        return (
          <ReasoningNodeView node={node} isStreaming={isStreaming} />
        );
      case "tool_step":
        return <ToolStepNodeView node={node} sessionId={sessionId} />;
      case "tool_group":
        return <ToolGroupNodeView node={node} sessionId={sessionId} />;
      case "approval":
        return <ApprovalNodeView node={node} />;
      case "iteration_boundary":
        return <IterationBoundaryView node={node} showDiagnostics={showDiagnostics} />;
      case "turn_status":
        return <TurnStatusView node={node} />;
      case "system_notice":
        return <SystemNoticeView node={node} />;
      default:
        return null;
    }
  })();

  if (content == null) return null;

  return (
    <div
      data-timeline-node-kind={node.kind}
      data-timeline-node-id={node.node_id}
      data-activity-row={isActivity ? "" : undefined}
      className="min-w-0 max-w-full"
    >
      {content}
    </div>
  );
}

// ============================================================================
// Helpers
// ============================================================================

function formatElapsed(ms: number): string {
  if (ms < 1000) return `${ms}ms`;
  const secs = ms / 1000;
  if (secs < 60) return `${secs.toFixed(1)}s`;
  const mins = Math.floor(secs / 60);
  const remSecs = Math.round(secs % 60);
  return `${mins}m ${remSecs}s`;
}
