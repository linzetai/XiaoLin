// TurnNodeRenderer — renders TurnDisplayNode[] from the canonical timeline store.
//
// Each TurnDisplayNode variant gets its own view component. Both live
// streaming and history replay use the same renderer (Decision D3).
//
// This is the canonical transcript renderer for both live and replayed turns.

import { memo, lazy, Suspense, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import type {
  TurnDisplayNode,
  UserMessageNode,
  AssistantTextNode,
  ToolStepNode,
  ToolGroupNode,
  ApprovalNode,
  IterationBoundaryNode,
  TurnStatusNode,
  SystemNoticeNode,
} from "../../lib/timeline/types";
import { normalizeApprovalDecision } from "../../lib/timeline/presentation";

import { UserInput } from "./UserInput";
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
  if (!node.content.trim()) return null;

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
          data-streaming-cursor
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
  const [expanded, setExpanded] = useState(false);
  const summary = useMemo(() => summarizeApproval(node), [node]);
  return (
    <div
      className="my-1.5 min-w-0 rounded-md border px-2 py-1 text-[12px]"
      style={{
        borderColor: summary.status === "pending"
          ? "color-mix(in srgb, var(--amber) 22%, transparent)"
          : "var(--separator)",
        background: summary.status === "pending"
          ? "color-mix(in srgb, var(--amber) 5%, transparent)"
          : "transparent",
        color: "var(--fill-secondary)",
      }}
      data-approval-row
    >
      <button
        type="button"
        className="flex w-full min-w-0 items-center gap-2 text-left"
        onClick={() => setExpanded((value) => !value)}
        aria-expanded={expanded}
      >
        <span
          className="h-1.5 w-1.5 shrink-0 rounded-full"
          style={{ background: summary.status === "pending" ? "var(--amber)" : "var(--green)" }}
        />
        <span className="min-w-0 flex-1 truncate font-medium">{summary.title}</span>
        {summary.target && (
          <span className="hidden min-w-0 max-w-[46%] truncate text-[11px] sm:inline" style={{ color: "var(--fill-quaternary)", fontFamily: "var(--font-mono)" }}>
            {summary.target}
          </span>
        )}
        <span className="shrink-0 text-[11px]" style={{ color: "var(--fill-quaternary)" }}>
          {formatElapsed(Math.max(0, node.updated_at_ms - node.created_at_ms))}
        </span>
      </button>
      {expanded && (
        <div className="mt-1 space-y-0.5 border-l pl-2 text-[11px]" style={{ borderColor: "var(--separator)", color: "var(--fill-tertiary)" }}>
          <div>动作：{node.action}</div>
          {node.reason && <div>原因：{node.reason}</div>}
          {node.decision && <div>决策：{node.decision}</div>}
          {node.decision_source && <div>来源：{node.decision_source}</div>}
        </div>
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
  const [detailsOpen, setDetailsOpen] = useState(false);

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
  const cause = classifyTurnFailure(node);
  const elapsed = node.elapsed_ms != null ? formatElapsed(node.elapsed_ms) : undefined;
  const diagnosticCode = usefulDiagnosisCode(node.diagnosis?.diagnosis_code);
  const diagnostic = JSON.stringify({
    turn_id: node.turn_id,
    end_reason: node.end_reason,
    status: node.status,
    elapsed_ms: node.elapsed_ms,
    diagnosis: node.diagnosis,
    summary: node.summary,
  }, null, 2);

  return (
    <div
      className="my-3 rounded-lg px-3 py-2 text-[12px]"
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
      data-turn-error-card
    >
      <div className="flex items-start gap-2">
        <span className="mt-[5px] inline-block h-[8px] w-[8px] shrink-0 rounded-full" style={{
          background: node.status === "failed" ? "var(--red)" : "var(--amber)",
        }} />
        <div className="min-w-0 flex-1">
          <div className="font-semibold" style={{ color: node.status === "failed" ? "var(--red)" : "var(--fill-secondary)" }}>
            本轮执行异常结束
          </div>
          <div className="mt-0.5 leading-relaxed" style={{ color: "var(--fill-tertiary)" }}>
            {message === "An unexpected error terminated the turn."
              ? "主流程在汇总或响应阶段中断；已完成的工具和子代理结果仍可展开查看。"
              : message}
          </div>
          <div className="mt-1 flex flex-wrap gap-x-3 gap-y-1 text-[11px]" style={{ color: "var(--fill-quaternary)" }}>
            <span>原因：{cause}</span>
            {elapsed && <span>耗时：{elapsed}</span>}
            {node.diagnosis?.tool_calls != null && <span>工具：{node.diagnosis.tool_calls} 次</span>}
            {diagnosticCode ? <span>Code：{diagnosticCode}</span> : <span>未获取到可用的诊断信息</span>}
          </div>
          <div className="mt-2 flex flex-wrap gap-2">
            <ErrorActionButton
              label="重试汇总"
              primary
              onClick={() => window.dispatchEvent(new CustomEvent("xiaolin:retry-summary", { detail: { turnId: node.turn_id } }))}
            />
            <ErrorActionButton label="查看详情" onClick={() => setDetailsOpen((value) => !value)} />
            <ErrorActionButton
              label="复制诊断信息"
              onClick={() => {
                void navigator.clipboard?.writeText(diagnostic);
              }}
            />
          </div>
          {detailsOpen && (
            <pre
              className="mt-2 max-h-[180px] overflow-auto whitespace-pre-wrap rounded-md p-2 text-[11px]"
              style={{
                background: "var(--bg-primary)",
                border: "0.5px solid var(--separator)",
                color: "var(--fill-tertiary)",
                fontFamily: "var(--font-mono)",
              }}
            >
              {diagnostic}
            </pre>
          )}
        </div>
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

function ErrorActionButton({ label, onClick, primary }: { label: string; onClick?: () => void; primary?: boolean }) {
  return (
    <button
      type="button"
      onClick={onClick}
      className="rounded px-2 py-0.5 text-[11px] transition-colors hover:bg-[var(--bg-hover)]"
      style={{
        border: "0.5px solid var(--separator)",
        color: primary ? "var(--bg-primary)" : "var(--fill-secondary)",
        background: primary ? "var(--fill-secondary)" : "transparent",
      }}
    >
      {label}
    </button>
  );
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
  /** The only text/reasoning node allowed to render a live cursor. */
  activeStreamingNodeId?: string | null;
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
  activeStreamingNodeId,
}: TurnNodeRendererProps) {
  if (nodes.length === 0) return null;
  const cursorNodeId = activeStreamingNodeId ?? selectActiveStreamingNodeId(nodes);

  return (
    <>
      {nodes.map((node) => (
        <TurnNodeView
          key={node.node_id}
          node={node}
          isLive={isLive}
          sessionId={sessionId}
          showDiagnostics={showDiagnostics}
          activeStreamingNodeId={cursorNodeId}
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
  activeStreamingNodeId,
}: {
  node: TurnDisplayNode;
  isLive?: boolean;
  sessionId?: string;
  showDiagnostics?: boolean;
  activeStreamingNodeId?: string | null;
}) {
  const isStreaming = isLive && node.status === "pending" && node.node_id === activeStreamingNodeId;
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
  if (
    (node.kind === "assistant_text" || node.kind === "reasoning") &&
    !node.content.trim()
  ) {
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
        return null;
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

interface ApprovalSummary {
  title: string;
  target?: string;
  status: "pending" | "completed";
}

function summarizeApproval(node: ApprovalNode): ApprovalSummary {
  const decision = normalizeApprovalDecision(node.decision);
  const target = extractApprovalTarget(node);
  if (!node.decision || node.status === "pending") {
    return {
      title: node.action.includes("command") ? "等待授权执行命令" : "等待授权",
      target,
      status: "pending",
    };
  }
  if (decision === "deny" || decision === "abort") {
    return {
      title: node.action.includes("command") ? "已拒绝执行命令" : "已拒绝授权",
      target,
      status: "completed",
    };
  }
  return {
    title: node.action.includes("command") ? "已批准执行命令" : "已获授权",
    target,
    status: "completed",
  };
}

function extractApprovalTarget(node: ApprovalNode): string | undefined {
  const text = [node.reason, node.action].filter(Boolean).join(" ");
  const commandMatch = text.match(/ShellCommand\s*\{\s*command:\s*"([^"]+)"/);
  if (commandMatch) return commandMatch[1];
  const quoted = text.match(/`([^`]+)`/);
  if (quoted) return quoted[1];
  if (node.reason && node.reason.length < 120) return node.reason;
  return undefined;
}

function classifyTurnFailure(node: TurnStatusNode): string {
  const code = node.diagnosis?.diagnosis_code?.toLowerCase() ?? "";
  const reason = node.end_reason.toLowerCase();
  if (code.includes("network") || reason.includes("network")) return "网络连接中断";
  if (code.includes("tool") || reason.includes("tool")) return "工具执行失败";
  if (code.includes("permission") || code.includes("approval") || reason.includes("cancelled")) return "权限或审批被拒绝";
  if (code.includes("model") || reason.includes("interrupted")) return "模型响应中断";
  if (code.includes("internal") || reason.includes("replaced")) return "内部错误";
  if (reason.includes("budget")) return "模型响应中断";
  return "未获取到诊断信息";
}

function usefulDiagnosisCode(code?: string): string | undefined {
  if (!code) return undefined;
  const normalized = code.toLowerCase();
  if (normalized === "error" || normalized === "unknown" || normalized === "unexpected_error") {
    return undefined;
  }
  return code;
}

function selectActiveStreamingNodeId(nodes: TurnDisplayNode[]): string | null {
  for (let index = nodes.length - 1; index >= 0; index -= 1) {
    const node = nodes[index];
    if (
      (node.kind === "assistant_text" || node.kind === "reasoning") &&
      node.status === "pending" &&
      node.content.trim()
    ) {
      return node.node_id;
    }
  }
  return null;
}
