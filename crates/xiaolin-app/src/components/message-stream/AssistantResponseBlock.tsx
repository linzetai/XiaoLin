// AssistantResponseBlock — visual container for the assistant's response nodes
// within a turn (Codex app / ChatGPT-style).
//
// Wraps TurnNodeRenderer so that tool steps, reasoning, and other assistant
// activity read as part of the assistant response rather than independent peer
// messages.
//
// Phase 4: Multi-ProcessInterval, attention items, running/completed tool separation.

import { memo, useMemo, useState, type ReactElement } from "react";
import { CaretRight, CheckCircle, CircleNotch, WarningCircle } from "@phosphor-icons/react";
import type { ToolStepNode, TurnDisplayNode } from "../../lib/timeline/types";
import {
  selectAssistantTurnPresentation,
  type AssistantPresentationItem,
  type ProcessInterval,
  type RuntimeActivityNarration,
} from "../../lib/timeline/presentation";
import { semanticToolTitle } from "./AssistantActivityGroup";
import { TurnNodeRenderer } from "./TurnNodeRenderer";

export interface AssistantResponseBlockProps {
  nodes: TurnDisplayNode[];
  /** When true, pending nodes show streaming animations. */
  isLive?: boolean;
  /** Session ID for sub-components that need it (tool output details). */
  sessionId?: string;
  /** When true, diagnostic-only timeline metadata can be visible. */
  showDiagnostics?: boolean;
}

/**
 * Assistant response block.
 *
 * Uses the canonical presentation builder for multi-interval and attention support.
 */
export const AssistantResponseBlock = memo(function AssistantResponseBlock({
  nodes,
  isLive,
  sessionId,
  showDiagnostics,
}: AssistantResponseBlockProps) {
  const presentation = useMemo(
    () => selectAssistantTurnPresentation(nodes, { showDiagnostics }),
    [nodes, showDiagnostics],
  );

  const items = presentation.items;
  const activeStreamingNodeId = useMemo(
    () => selectActiveStreamingNodeId(nodes),
    [nodes],
  );

  if (items.length === 0) return null;

  const processItems = items.filter((item) => !isCompletedAnswerItem(item));
  const shouldFoldCompletedProcess =
    presentation.mode === "completed" && processItems.length > 0;

  const renderItem = (item: AssistantPresentationItem) => (
    <PresentationItemView
      key={presentationItemKey(item)}
      item={item}
      isLive={isLive}
      sessionId={sessionId}
      showDiagnostics={showDiagnostics}
      activeStreamingNodeId={activeStreamingNodeId}
    />
  );

  return (
    <div
      className="assistant-response min-w-0 w-full max-w-full"
      data-diagnostics={showDiagnostics ? "true" : undefined}
      data-presentation-mode={presentation.mode}
    >
      {shouldFoldCompletedProcess ? (
        renderCompletedTurnItems({
          items,
          processItems,
          durationMs: presentation.terminalStatus?.elapsed_ms,
          renderItem,
          isLive,
          sessionId,
          showDiagnostics,
          activeStreamingNodeId,
        })
      ) : (
        items.map(renderItem)
      )}
    </div>
  );
});

function PresentationItemView({
  item,
  isLive,
  sessionId,
  showDiagnostics,
  activeStreamingNodeId,
}: {
  item: AssistantPresentationItem;
  isLive?: boolean;
  sessionId?: string;
  showDiagnostics?: boolean;
  activeStreamingNodeId?: string | null;
}) {
  if (item.kind === "narration") {
    return <NarrationRow narration={item.narration} />;
  }
  if (item.kind === "completed_batch" || item.kind === "process_interval") {
    return (
      <CompletedProcessInterval
        interval={item.interval}
        isLive={isLive}
        sessionId={sessionId}
        showDiagnostics={showDiagnostics}
        activeStreamingNodeId={activeStreamingNodeId}
      />
    );
  }
  if (
    item.kind === "running_tool" ||
    item.kind === "failed_tool" ||
    item.kind === "approval" ||
    item.kind === "error"
  ) {
    return (
      <TurnNodeRenderer
        nodes={[item.node]}
        isLive={isLive}
        sessionId={sessionId}
        showDiagnostics={showDiagnostics}
        activeStreamingNodeId={activeStreamingNodeId}
      />
    );
  }
  if (item.kind === "attention") {
    return (
      <AttentionRow
        node={item.node}
        isLive={isLive}
        sessionId={sessionId}
        showDiagnostics={showDiagnostics}
        activeStreamingNodeId={activeStreamingNodeId}
      />
    );
  }
  return (
    <TurnNodeRenderer
      nodes={[item.node]}
      isLive={isLive}
      sessionId={sessionId}
      showDiagnostics={showDiagnostics}
      activeStreamingNodeId={activeStreamingNodeId}
    />
  );
}

function isCompletedAnswerItem(item: AssistantPresentationItem): boolean {
  return item.kind === "visible" && item.node.kind === "assistant_text";
}

function renderCompletedTurnItems({
  items,
  processItems,
  durationMs,
  renderItem,
  isLive,
  sessionId,
  showDiagnostics,
  activeStreamingNodeId,
}: {
  items: AssistantPresentationItem[];
  processItems: AssistantPresentationItem[];
  durationMs?: number;
  renderItem: (item: AssistantPresentationItem) => ReactElement;
  isLive?: boolean;
  sessionId?: string;
  showDiagnostics?: boolean;
  activeStreamingNodeId?: string | null;
}): ReactElement[] {
  const rendered: ReactElement[] = [];
  let emittedProcessSummary = false;

  for (const item of items) {
    if (isCompletedAnswerItem(item)) {
      rendered.push(renderItem(item));
      continue;
    }
    if (!emittedProcessSummary) {
      rendered.push(
        <CompletedTurnProcessSummary
          key="completed-turn-process"
          items={processItems}
          durationMs={durationMs}
          isLive={isLive}
          sessionId={sessionId}
          showDiagnostics={showDiagnostics}
          activeStreamingNodeId={activeStreamingNodeId}
        />,
      );
      emittedProcessSummary = true;
    }
  }

  return rendered;
}

function presentationItemKey(item: AssistantPresentationItem): string {
  if (item.kind === "narration") return item.narration.id;
  if (item.kind === "completed_batch" || item.kind === "process_interval") return item.interval.id;
  return item.node.node_id;
}

function CompletedTurnProcessSummary({
  items,
  durationMs,
  isLive,
  sessionId,
  showDiagnostics,
  activeStreamingNodeId,
}: {
  items: AssistantPresentationItem[];
  durationMs?: number;
  isLive?: boolean;
  sessionId?: string;
  showDiagnostics?: boolean;
  activeStreamingNodeId?: string | null;
}) {
  const [expanded, setExpanded] = useState(false);
  const duration = durationMs != null ? formatProcessDuration(durationMs) : inferItemsDuration(items);

  return (
    <div
      className="completed-turn-process my-2 min-w-0"
      data-presentation-kind="completed_turn_process"
      data-activity-row
    >
      <button
        type="button"
        onClick={() => setExpanded((value) => !value)}
        className="flex w-full min-w-0 items-center gap-2 border-b px-0 py-2 text-left transition-colors duration-100"
        style={{
          borderColor: "var(--separator)",
          color: "var(--fill-tertiary)",
        }}
        aria-expanded={expanded}
      >
        <span className="flex h-[16px] w-[16px] shrink-0 items-center justify-center">
          <CheckCircle size={13} weight="fill" style={{ color: "var(--fill-quaternary)" }} />
        </span>
        <span className="min-w-0 flex-1 truncate text-[13px] font-medium" style={{ color: "var(--fill-tertiary)" }}>
          {duration ? `已处理 ${duration}` : "已处理"}
        </span>
        <CaretRight
          size={12}
          className="shrink-0 transition-transform duration-150"
          style={{
            color: "var(--fill-quaternary)",
            transform: expanded ? "rotate(90deg)" : "rotate(0)",
          }}
        />
      </button>
      {expanded && (
        <div
          className="mt-2 min-w-0 border-l pl-3"
          style={{ borderColor: "var(--separator)" }}
          data-completed-turn-process-transcript
        >
          {items.map((item) => (
            <PresentationItemView
              key={presentationItemKey(item)}
              item={item}
              isLive={isLive}
              sessionId={sessionId}
              showDiagnostics={showDiagnostics}
              activeStreamingNodeId={activeStreamingNodeId}
            />
          ))}
        </div>
      )}
    </div>
  );
}

// ============================================================================
// NarrationRow
// ============================================================================

function NarrationRow({ narration }: { narration: RuntimeActivityNarration }) {
  return (
    <div
      className="py-1.5 text-[14px] leading-7"
      style={{ color: "var(--fill-primary)" }}
      data-activity-narration={narration.source}
      data-activity-phase={narration.phaseKey}
    >
      {narration.text}
    </div>
  );
}

// ============================================================================
// CompletedProcessInterval
// ============================================================================

function CompletedProcessInterval({
  interval,
  isLive,
  sessionId,
  showDiagnostics,
  activeStreamingNodeId,
}: {
  interval: ProcessInterval;
  isLive?: boolean;
  sessionId?: string;
  showDiagnostics?: boolean;
  activeStreamingNodeId?: string | null;
}) {
  const [expanded, setExpanded] = useState(false);

  if (interval.nodes.length === 0) return null;
  const summary = summarizeProcessInterval(interval);
  const detailNodes = interval.nodes.filter(shouldRenderProcessDetailNode);

  return (
    <div
      className="completed-process-summary my-3 min-w-0"
      data-completed-process-interval={interval.id}
      data-presentation-kind="process_interval"
      data-activity-row
    >
      <button
        type="button"
        onClick={() => setExpanded((value) => !value)}
        className="flex w-full min-w-0 items-center gap-2 border-b px-0 py-2 text-left transition-colors duration-100"
        style={{
          borderColor: "var(--separator)",
          color: "var(--fill-tertiary)",
        }}
        aria-expanded={expanded}
      >
        <span className="flex h-[16px] w-[16px] shrink-0 items-center justify-center">
          <ProcessStatusIcon status={summary.status} />
        </span>
        <span className="min-w-0 flex-1 truncate text-[13px] font-medium" style={{ color: "var(--fill-tertiary)" }}>
          {summary.title}
        </span>
        <CaretRight
          size={12}
          className="shrink-0 transition-transform duration-150"
          style={{
            color: "var(--fill-quaternary)",
            transform: expanded ? "rotate(90deg)" : "rotate(0)",
          }}
        />
        {summary.duration && (
          <span className="shrink-0 text-[11px] tabular-nums" style={{ color: "var(--fill-quaternary)" }}>
            {summary.duration}
          </span>
        )}
      </button>
      {expanded && detailNodes.length > 0 && (
        <div
          className="mt-2 min-w-0 border-l pl-3"
          style={{ borderColor: "var(--separator)" }}
          data-completed-process-transcript
        >
          {detailNodes.map((node) => (
            <TurnNodeRenderer
              key={node.node_id}
              nodes={[node]}
              isLive={isLive}
              sessionId={sessionId}
              showDiagnostics={showDiagnostics}
              activeStreamingNodeId={activeStreamingNodeId}
            />
          ))}
        </div>
      )}
    </div>
  );
}

// ============================================================================
// AttentionRow
// ============================================================================

function AttentionRow({
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
  if (node.kind === "turn_status" || node.kind === "tool_step") {
    return (
      <TurnNodeRenderer
        nodes={[node]}
        isLive={isLive}
        sessionId={sessionId}
        showDiagnostics={showDiagnostics}
        activeStreamingNodeId={activeStreamingNodeId}
      />
    );
  }

  return (
    <div
      className="my-2 flex items-start gap-2 rounded-lg px-3 py-2 text-[12px]"
      style={{
        background: "color-mix(in srgb, var(--amber) 8%, transparent)",
        border: "0.5px solid color-mix(in srgb, var(--amber) 25%, transparent)",
        color: "var(--fill-secondary)",
      }}
      data-attention-row
    >
      <WarningCircle size={14} weight="fill" style={{ color: "var(--amber)", marginTop: 1 }} />
      <div className="flex-1 min-w-0">
        <TurnNodeRenderer
          nodes={[node]}
          isLive={isLive}
          sessionId={sessionId}
          showDiagnostics={showDiagnostics}
          activeStreamingNodeId={activeStreamingNodeId}
        />
      </div>
    </div>
  );
}

function formatProcessDuration(ms: number): string {
  if (ms < 1000) return `${ms}ms`;
  const secs = ms / 1000;
  if (secs < 60) return `${Math.round(secs)}s`;
  const mins = Math.floor(secs / 60);
  const remSecs = Math.round(secs % 60);
  return `${mins}m ${remSecs}s`;
}

function inferItemsDuration(items: AssistantPresentationItem[]): string | undefined {
  const durations = items.flatMap((item) => {
    if (item.kind === "completed_batch" || item.kind === "process_interval") {
      return [item.interval.durationMs];
    }
    if (
      item.kind === "running_tool" ||
      item.kind === "failed_tool" ||
      item.kind === "approval" ||
      item.kind === "error" ||
      item.kind === "attention" ||
      item.kind === "visible"
    ) {
      if ("duration_ms" in item.node && typeof item.node.duration_ms === "number") {
        return [item.node.duration_ms];
      }
    }
    return [];
  });
  const total = durations.reduce((sum, duration) => sum + Math.max(0, duration), 0);
  return total > 0 ? formatProcessDuration(total) : undefined;
}

type ProcessStatus = "running" | "completed" | "failed";

interface ProcessSummary {
  title: string;
  duration?: string;
  status: ProcessStatus;
}

function summarizeProcessInterval(interval: ProcessInterval, options: { isLive?: boolean } = {}): ProcessSummary {
  const toolSteps = collectToolSteps(interval.nodes);
  const completedCount = toolSteps.filter((step) => step.status === "completed").length;
  const activeTool = [...toolSteps]
    .reverse()
    .find((step) => step.status === "running" || step.status === "pending");
  const hasFailed = toolSteps.some((step) => step.status === "failed" || step.status === "cancelled");
  const hasPendingNode = interval.nodes.some((node) => node.status === "pending" || node.status === "running");
  const status: ProcessStatus = activeTool || hasPendingNode || options.isLive
    ? "running"
    : hasFailed
      ? "failed"
      : "completed";
  const duration = interval.durationMs > 0 ? formatProcessDuration(interval.durationMs) : undefined;

  if (status === "running") {
    return {
      title: activeTool
        ? joinStatusParts(`正在运行 ${runningToolLabel(activeTool)}`, completedCount > 0 ? `已完成 ${completedCount} 条` : undefined)
        : toolSteps.length > 0
          ? `正在思考 · ${completedBatchTitle(toolSteps)}`
          : "正在思考",
      duration,
      status,
    };
  }

  if (toolSteps.length > 0) {
    return {
      title: status === "failed" ? `${completedBatchTitle(toolSteps)}，部分失败` : completedBatchTitle(toolSteps),
      duration,
      status,
    };
  }

  return {
    title: "已处理",
    duration,
    status,
  };
}

function joinStatusParts(primary: string, secondary?: string): string {
  return secondary ? `${primary} · ${secondary}` : primary;
}

function ProcessStatusIcon({ status }: { status: ProcessStatus }) {
  if (status === "running") {
    return <CircleNotch size={13} className="animate-spin" style={{ color: "var(--tint)" }} />;
  }
  if (status === "failed") {
    return <WarningCircle size={13} weight="fill" style={{ color: "var(--red)" }} />;
  }
  return <CheckCircle size={13} weight="fill" style={{ color: "var(--fill-quaternary)" }} />;
}

function collectToolSteps(nodes: ProcessInterval["nodes"]): ToolStepNode[] {
  return nodes.flatMap((node) => {
    if (node.kind === "tool_step") return [node];
    if (node.kind === "tool_group") return node.steps;
    return [];
  });
}

function completedBatchTitle(steps: ToolStepNode[]): string {
  if (steps.length === 0) return "已处理";
  const categories = new Set(steps.map((step) => step.tool_category ?? "other"));
  if (categories.size === 1) {
    const category = steps[0].tool_category;
    if (category === "file") return `已读取 ${steps.length} 个文件`;
    if (category === "search") return `已搜索 ${steps.length} 次`;
    if (category === "shell") return `已运行 ${steps.length} 条命令`;
    if (category === "sub_agent") return `已完成 ${steps.length} 个子代理任务`;
  }
  return `已完成 ${steps.length} 个工具步骤`;
}

function runningToolLabel(step: ToolStepNode): string {
  const command = (step.target?.command ?? readStringArg(step.args, "command"))?.trim();
  if (step.tool_category === "shell" && command) {
    return truncate(command, 96);
  }
  return semanticToolTitle(step);
}

function readStringArg(args: string | undefined, key: string): string | undefined {
  if (!args) return undefined;
  try {
    const parsed = JSON.parse(args) as Record<string, unknown>;
    const value = parsed[key];
    return typeof value === "string" ? value : undefined;
  } catch {
    return undefined;
  }
}

function shouldRenderProcessDetailNode(node: ProcessInterval["nodes"][number]): boolean {
  void node;
  return true;
}

function truncate(value: string, max: number): string {
  return value.length > max ? `${value.slice(0, max - 1)}…` : value;
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
