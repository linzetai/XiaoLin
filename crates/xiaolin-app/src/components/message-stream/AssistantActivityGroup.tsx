import { memo, useMemo, useState } from "react";
import { CaretRight, CheckCircle, CircleNotch, WarningCircle } from "@phosphor-icons/react";
import type { ToolGroupNode, ToolStepNode } from "../../lib/timeline/types";
import { ICON_SIZE } from "../../lib/ui-tokens";
import { ToolStepView } from "./ToolStepView";

export type ToolActivityNode = ToolStepNode | ToolGroupNode;

export interface AssistantActivityGroupProps {
  nodes: ToolActivityNode[];
  sessionId?: string;
  defaultExpanded?: boolean;
}

export const AssistantActivityGroup = memo(function AssistantActivityGroup({
  nodes,
  sessionId,
  defaultExpanded = false,
}: AssistantActivityGroupProps) {
  const [expanded, setExpanded] = useState(defaultExpanded);
  const summary = useMemo(() => summarizeActivity(nodes), [nodes]);
  if (nodes.length === 0) return null;

  return (
    <div
      data-activity-row
      data-presentation-kind="tool_activity_group"
      className="assistant-activity-group my-1 min-w-0 max-w-full"
    >
      <button
        type="button"
        onClick={() => setExpanded((value) => !value)}
        className="flex w-full min-w-0 items-center gap-2 rounded-md px-1 py-0.5 text-left transition-colors duration-100"
        style={{ color: "var(--fill-tertiary)" }}
        aria-expanded={expanded}
      >
        <CaretRight
          size={12}
          className="shrink-0 transition-transform duration-150"
          style={{
            color: "var(--fill-quaternary)",
            transform: expanded ? "rotate(90deg)" : "rotate(0)",
          }}
        />
        <span className="flex h-[16px] w-[16px] shrink-0 items-center justify-center">
          <ActivityStatusIcon status={summary.status} />
        </span>
        <span className="min-w-0 flex-1 truncate text-[12px] font-medium" style={{ color: "var(--fill-secondary)" }}>
          {summary.title}
        </span>
        {summary.detail && (
          <span className="hidden min-w-0 max-w-[38%] truncate text-[11px] sm:inline" style={{ color: "var(--fill-quaternary)" }}>
            {summary.detail}
          </span>
        )}
        {summary.duration && (
          <span className="shrink-0 text-[10px] tabular-nums" style={{ color: "var(--fill-quaternary)" }}>
            {summary.duration}
          </span>
        )}
      </button>
      {expanded && (
        <div className="ml-[28px] border-l pl-2" style={{ borderColor: "var(--separator)" }}>
          {nodes.map((node) =>
            node.kind === "tool_group" ? (
              <div key={node.node_id} data-timeline-node-kind={node.kind} data-timeline-node-id={node.node_id}>
                {node.steps.map((step) => (
                  <ToolStepView
                    key={step.node_id}
                    node={step}
                    sessionId={sessionId}
                    presentationTitle={semanticToolTitle(step)}
                  />
                ))}
              </div>
            ) : (
              <div key={node.node_id} data-timeline-node-kind={node.kind} data-timeline-node-id={node.node_id}>
                <ToolStepView
                  node={node}
                  sessionId={sessionId}
                  presentationTitle={semanticToolTitle(node)}
                />
              </div>
            ),
          )}
        </div>
      )}
    </div>
  );
});

function ActivityStatusIcon({ status }: { status: ActivitySummary["status"] }) {
  if (status === "running") {
    return <CircleNotch size={ICON_SIZE.sm} className="animate-spin" style={{ color: "var(--tint)" }} />;
  }
  if (status === "failed") {
    return <WarningCircle size={ICON_SIZE.sm} weight="fill" style={{ color: "var(--red)" }} />;
  }
  return <CheckCircle size={ICON_SIZE.sm} weight="fill" style={{ color: "var(--green)" }} />;
}

interface ActivitySummary {
  title: string;
  detail?: string;
  duration?: string;
  status: "running" | "completed" | "failed";
}

function summarizeActivity(nodes: ToolActivityNode[]): ActivitySummary {
  const steps = nodes.flatMap((node) => node.kind === "tool_group" ? node.steps : [node]);
  const status = steps.some((step) => step.status === "running" || step.status === "pending")
    ? "running"
    : steps.some((step) => step.status === "failed" || step.status === "cancelled")
      ? "failed"
      : "completed";
  const durationMs = steps.reduce((total, step) => total + (step.duration_ms ?? 0), 0);
  const duration = durationMs > 0 ? formatDuration(durationMs) : undefined;

  if (steps.every(isSubAgentTool)) {
    return {
      title: status === "running" ? "Reviewing with sub-agents" : `Reviewed with ${steps.length} sub-agent${steps.length === 1 ? "" : "s"}`,
      detail: summarizeTargets(steps),
      duration,
      status,
    };
  }

  if (steps.every(isDiffTool)) {
    return {
      title: steps.length === 1 ? "Inspect changed files" : `Inspected changes with ${steps.length} commands`,
      detail: summarizeTargets(steps),
      duration,
      status,
    };
  }

  const categories = new Set(steps.map((step) => step.tool_category ?? "other"));
  const title = steps.length === 1
    ? semanticToolTitle(steps[0])
    : categories.size === 1
      ? `${verbForCategory(steps[0].tool_category)} ${steps.length} items`
      : `Ran ${steps.length} activity steps`;

  return {
    title,
    detail: summarizeTargets(steps),
    duration,
    status,
  };
}

export function semanticToolTitle(node: ToolStepNode): string {
  const name = node.tool_name.toLowerCase();
  const target = node.target;
  const command = target?.command ?? "";

  if (isSubAgentTool(node)) {
    if (name.includes("get")) return "Collect sub-agent result";
    return "Start sub-agent review";
  }
  if (isDiffTool(node)) return "Inspect changed files";
  if (name === "skill" || node.display_title.toLowerCase() === "skill") return "Load instructions";
  if (node.tool_category === "search") return target?.query ? "Search codebase" : "Search files";
  if (node.tool_category === "file") return target?.path ? "Read file" : "Inspect file";
  if (node.tool_category === "shell") {
    if (command.startsWith("git status")) return "Check repository status";
    if (command.startsWith("git diff")) return "Inspect changed files";
    if (command.includes("test")) return "Run tests";
    if (command.includes("build")) return "Build app";
    return "Run command";
  }
  if (node.tool_category === "web") return "Check web source";
  if (node.tool_category === "mcp") return "Call connected app";
  return cleanDisplayTitle(node.display_title);
}

function summarizeTargets(steps: ToolStepNode[]): string | undefined {
  const values = steps
    .map((step) => step.target?.path ?? step.target?.query ?? step.target?.url ?? step.target?.label ?? step.target?.command)
    .filter((value): value is string => Boolean(value));
  if (values.length === 0) return undefined;
  const first = values[0];
  return values.length === 1 ? truncate(first, 80) : `${truncate(first, 54)} +${values.length - 1}`;
}

function isSubAgentTool(node: ToolStepNode): boolean {
  const name = node.tool_name.toLowerCase();
  return node.tool_category === "sub_agent" || name.includes("subagent") || name.includes("sub_agent");
}

function isDiffTool(node: ToolStepNode): boolean {
  const command = node.target?.command?.toLowerCase() ?? "";
  const title = node.display_title.toLowerCase();
  return command.startsWith("git diff") || title.includes("git diff");
}

function verbForCategory(category?: string): string {
  switch (category) {
    case "file":
      return "Inspected";
    case "search":
      return "Searched";
    case "shell":
      return "Ran";
    case "sub_agent":
      return "Reviewed with";
    default:
      return "Processed";
  }
}

function cleanDisplayTitle(title: string): string {
  return title.replace(/\s+/g, " ").replace(/^Run\s+/i, "").trim() || "Tool activity";
}

function truncate(value: string, max: number): string {
  return value.length > max ? `${value.slice(0, max - 1)}…` : value;
}

function formatDuration(ms: number): string {
  if (ms < 1000) return `${ms}ms`;
  return `${(ms / 1000).toFixed(1)}s`;
}
