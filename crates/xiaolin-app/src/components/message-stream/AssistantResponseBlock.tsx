// AssistantResponseBlock — visual container for the assistant's response nodes
// within a turn (Codex app / ChatGPT-style).
//
// Wraps TurnNodeRenderer so that tool steps, reasoning, and other assistant
// activity read as part of the assistant response rather than independent peer
// messages.

import { memo, useMemo, useState } from "react";
import { CaretRight, CheckCircle } from "@phosphor-icons/react";
import type { ToolGroupNode, ToolStepNode, TurnDisplayNode } from "../../lib/timeline/types";
import { selectAssistantTurnPresentation, type AssistantProcessNode } from "../../lib/timeline/presentation";
import { AssistantActivityGroup, type ToolActivityNode } from "./AssistantActivityGroup";
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
 * The block itself stays visually quiet; individual reasoning/tool/status rows
 * carry the secondary activity styling so assistant text remains the primary
 * narrative.
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
  const items = useMemo(
    () => buildPresentationItems(presentation.items),
    [presentation.items],
  );
  if (items.length === 0) return null;

  return (
    <div
      className="assistant-response min-w-0 w-full max-w-full"
      data-diagnostics={showDiagnostics ? "true" : undefined}
      data-presentation-mode={presentation.mode}
    >
      {items.map((item) => {
        if (item.kind === "tool_activity_group") {
          return (
            <AssistantActivityGroup
              key={item.id}
              nodes={item.nodes}
              sessionId={sessionId}
            />
          );
        }
        if (item.kind === "process_summary") {
          return (
            <CompletedProcessSummary
              key={item.id}
              id={item.id}
              nodes={item.nodes}
              elapsedMs={item.elapsedMs}
              isLive={isLive}
              sessionId={sessionId}
              showDiagnostics={showDiagnostics}
            />
          );
        }
        return (
          <TurnNodeRenderer
            key={item.node.node_id}
            nodes={[item.node]}
            isLive={isLive}
            sessionId={sessionId}
            showDiagnostics={showDiagnostics}
          />
        );
      })}
    </div>
  );
});

type PresentationItem =
  | { kind: "node"; node: TurnDisplayNode }
  | { kind: "tool_activity_group"; id: string; nodes: ToolActivityNode[] }
  | { kind: "process_summary"; id: string; nodes: AssistantProcessNode[]; elapsedMs?: number };

function buildPresentationItems(
  sourceItems: ReturnType<typeof selectAssistantTurnPresentation>["items"],
): PresentationItem[] {
  const items: PresentationItem[] = [];
  let pendingTools: ToolActivityNode[] = [];
  let pendingFamily: string | null = null;

  const flushTools = () => {
    if (pendingTools.length === 0) return;
    items.push({
      kind: "tool_activity_group",
      id: `tool-activity-${pendingTools[0].node_id}-${pendingTools.length}`,
      nodes: pendingTools,
    });
    pendingTools = [];
    pendingFamily = null;
  };

  for (const sourceItem of sourceItems) {
    if (sourceItem.kind === "process_summary") {
      flushTools();
      items.push(sourceItem);
      continue;
    }

    const node = sourceItem.node;
    if (node.kind === "tool_step" || node.kind === "tool_group") {
      const family = toolActivityFamily(node as ToolStepNode | ToolGroupNode);
      if (pendingFamily != null && pendingFamily !== family) {
        flushTools();
      }
      pendingTools.push(node as ToolStepNode | ToolGroupNode);
      pendingFamily = family;
      continue;
    }
    flushTools();
    items.push({ kind: "node", node });
  }

  flushTools();
  return items;
}

function CompletedProcessSummary({
  id,
  nodes,
  elapsedMs,
  isLive,
  sessionId,
  showDiagnostics,
}: {
  id: string;
  nodes: AssistantProcessNode[];
  elapsedMs?: number;
  isLive?: boolean;
  sessionId?: string;
  showDiagnostics?: boolean;
}) {
  const [expanded, setExpanded] = useState(false);
    const expandedItems = useMemo(
    () => buildPresentationItems(
      nodes.map((node) => ({ kind: "visible" as const, node })),
    ),
    [nodes],
  );

  if (nodes.length === 0) return null;

  return (
    <div
      className="completed-process-summary my-3 min-w-0"
      data-completed-process-summary={id}
      data-presentation-kind="process_summary"
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
        <span className="shrink-0 text-[13px] font-medium" style={{ color: "var(--fill-tertiary)" }}>
          已处理{elapsedMs != null ? ` ${formatProcessDuration(elapsedMs)}` : ""}
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
          data-completed-process-transcript
        >
          {expandedItems.map((item) => {
            if (item.kind === "tool_activity_group") {
              return (
                <AssistantActivityGroup
                  key={item.id}
                  nodes={item.nodes}
                  sessionId={sessionId}
                  defaultExpanded
                />
              );
            }
            if (item.kind === "process_summary") return null;
            return (
              <TurnNodeRenderer
                key={item.node.node_id}
                nodes={[item.node]}
                isLive={isLive}
                sessionId={sessionId}
                showDiagnostics={showDiagnostics}
              />
            );
          })}
        </div>
      )}
    </div>
  );
}

function toolActivityFamily(node: ToolActivityNode): string {
  const step = node.kind === "tool_group" ? node.steps[0] : node;
  if (!step) return "tool:empty";
  const name = step.tool_name.toLowerCase();
  const command = step.target?.command?.toLowerCase() ?? "";
  const title = step.display_title.toLowerCase();
  if (step.tool_category === "sub_agent" || name.includes("subagent") || name.includes("sub_agent")) {
    return "tool:sub_agent";
  }
  if (command.startsWith("git diff") || title.includes("git diff")) {
    return "tool:diff";
  }
  return `tool:${step.tool_category ?? "other"}`;
}

function formatProcessDuration(ms: number): string {
  if (ms < 1000) return `${ms}ms`;
  const secs = ms / 1000;
  if (secs < 60) return `${Math.round(secs)}s`;
  const mins = Math.floor(secs / 60);
  const remSecs = Math.round(secs % 60);
  return `${mins}m ${remSecs}s`;
}
