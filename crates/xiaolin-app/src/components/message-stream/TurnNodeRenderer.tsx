// TurnNodeRenderer — renders TurnDisplayNode[] from the canonical timeline store.
//
// Each TurnDisplayNode variant gets its own view component. Both live
// streaming and history replay use the same renderer (Decision D3).
//
// This component coexists with the legacy MessageRendererRow during the
// Phase 5 migration. Once Phase 7 removes the legacy path, this becomes
// the sole transcript renderer.

import { memo, lazy, Suspense } from "react";
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
import { StepIndicator } from "./StepIndicator";
import { StepGroup } from "./StepGroup";
import { ReasoningBlock } from "./ReasoningBlock";

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
    <div className="pb-1">
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
}: {
  node: ToolStepNode;
}) {
  const toolCall = {
    id: node.call_id,
    name: node.tool_name,
    status: mapNodeStatusToToolStatus(node.status),
    args: node.args,
    result: node.output_preview?.content ?? node.error_message,
    displayOutput: node.output_preview?.content,
    duration: node.duration_ms,
    metadata: node.target
      ? { target: node.target }
      : undefined,
    outputHandle: node.output_detail?.handle,
    outputSizeClass: node.output_detail?.size_class,
    outputIsExpandable: node.output_detail?.is_expandable,
  };

  return <StepIndicator tool={toolCall} />;
});

function mapNodeStatusToToolStatus(
  status: string,
): "running" | "success" | "error" {
  switch (status) {
    case "running":
      return "running";
    case "failed":
      return "error";
    case "cancelled":
      return "error";
    default:
      return "success";
  }
}

// ============================================================================
// ToolGroupNodeView
// ============================================================================

const ToolGroupNodeView = memo(function ToolGroupNodeView({
  node,
}: {
  node: ToolGroupNode;
}) {
  if (node.steps.length === 0) return null;

  const tools = node.steps.map((step) => ({
    id: step.call_id,
    name: step.tool_name,
    status: mapNodeStatusToToolStatus(step.status),
    args: step.args,
    result: step.output_preview?.content ?? step.error_message,
    displayOutput: step.output_preview?.content,
    duration: step.duration_ms,
    metadata: step.target ? { target: step.target } : undefined,
    outputHandle: step.output_detail?.handle,
    outputSizeClass: step.output_detail?.size_class,
    outputIsExpandable: step.output_detail?.is_expandable,
  }));

  return (
    <StepGroup
      tools={tools}
      streaming={node.status === "running"}
    />
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
    <div className="my-1 text-[11px]" style={{ color: "var(--fill-tertiary)" }}>
      <span className="mr-1">🔒</span>
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
  node: _node,
}: {
  node: IterationBoundaryNode;
}) {
  return (
    <div
      className="flex items-center justify-center gap-1.5 my-3 select-none"
      style={{ maxHeight: 32 }}
    >
      <span
        className="inline-block h-[4px] w-[4px] rounded-full"
        style={{ background: "var(--fill-quaternary)", opacity: 0.5 }}
      />
      <span
        className="inline-block h-[4px] w-[4px] rounded-full"
        style={{ background: "var(--fill-quaternary)", opacity: 0.5 }}
      />
      <span
        className="inline-block h-[4px] w-[4px] rounded-full"
        style={{ background: "var(--fill-quaternary)", opacity: 0.5 }}
      />
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
        overflowWrap: "anywhere",
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
  sessionId: _sessionId,
}: TurnNodeRendererProps) {
  if (nodes.length === 0) return null;

  return (
    <>
      {nodes.map((node) => (
        <TurnNodeView
          key={node.node_id}
          node={node}
          isLive={isLive}
        />
      ))}
    </>
  );
});

function TurnNodeView({
  node,
  isLive,
}: {
  node: TurnDisplayNode;
  isLive?: boolean;
}) {
  const isStreaming = isLive && node.status === "pending";

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
      return <ToolStepNodeView node={node} />;
    case "tool_group":
      return <ToolGroupNodeView node={node} />;
    case "approval":
      return <ApprovalNodeView node={node} />;
    case "iteration_boundary":
      return <IterationBoundaryView node={node} />;
    case "turn_status":
      return <TurnStatusView node={node} />;
    case "system_notice":
      return <SystemNoticeView node={node} />;
    default:
      return null;
  }
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
