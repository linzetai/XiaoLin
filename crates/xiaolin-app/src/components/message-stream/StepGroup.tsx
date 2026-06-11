import { useState, useMemo, memo, useCallback } from "react";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
import { CaretRight } from "@phosphor-icons/react";
import { StepIndicator, type ToolCall } from "./StepIndicator";
import type { StreamSegment } from "./types";
import { isTodoResult } from "./TodoCard";
import { isEditResult } from "./DiffCard";
const DEFAULT_THRESHOLD = 3;

export interface StepGroupItem {
  type: "single-tool";
  segment: StreamSegment;
}

export interface StepGroupCluster {
  type: "tool-group";
  segments: StreamSegment[];
}

export interface TextItem {
  type: "text";
  segment: StreamSegment;
}

export type GroupedSegment = StepGroupItem | StepGroupCluster | TextItem;

function isSpecialToolCall(tc: ToolCall): boolean {
  if (!tc.result) return false;
  if (isTodoResult(tc.name, tc.result)) return true;
  if (isEditResult(tc.name, tc.result)) return true;
  return false;
}

export function groupConsecutiveSegments(
  segments: StreamSegment[],
  threshold = DEFAULT_THRESHOLD,
): GroupedSegment[] {
  const result: GroupedSegment[] = [];
  let toolBuffer: StreamSegment[] = [];

  const flushBuffer = () => {
    if (toolBuffer.length === 0) return;
    if (toolBuffer.length >= threshold) {
      result.push({ type: "tool-group", segments: [...toolBuffer] });
    } else {
      for (const seg of toolBuffer) {
        result.push({ type: "single-tool", segment: seg });
      }
    }
    toolBuffer = [];
  };

  for (const seg of segments) {
    if (seg.type === "text") {
      flushBuffer();
      result.push({ type: "text", segment: seg });
    } else if (seg.type === "tool" && seg.toolCall) {
      if (isSpecialToolCall(seg.toolCall)) {
        flushBuffer();
        result.push({ type: "single-tool", segment: seg });
      } else {
        toolBuffer.push(seg);
      }
    }
  }
  flushBuffer();
  return result;
}

export function groupConsecutiveToolCalls(
  toolCalls: ToolCall[],
  threshold = DEFAULT_THRESHOLD,
): Array<{ type: "single"; tool: ToolCall } | { type: "group"; tools: ToolCall[] }> {
  const result: Array<{ type: "single"; tool: ToolCall } | { type: "group"; tools: ToolCall[] }> = [];
  let buffer: ToolCall[] = [];

  const flushBuffer = () => {
    if (buffer.length === 0) return;
    if (buffer.length >= threshold) {
      result.push({ type: "group", tools: [...buffer] });
    } else {
      for (const tc of buffer) {
        result.push({ type: "single", tool: tc });
      }
    }
    buffer = [];
  };

  for (const tc of toolCalls) {
    if (isSpecialToolCall(tc)) {
      flushBuffer();
      result.push({ type: "single", tool: tc });
    } else {
      buffer.push(tc);
    }
  }
  flushBuffer();
  return result;
}

interface GroupSummary {
  typeDistribution: Record<string, number>;
  totalDuration: number;
  errorCount: number;
  runningCount: number;
  successCount: number;
}

function computeSummary(tools: ToolCall[]): GroupSummary {
  const typeDistribution: Record<string, number> = {};
  let totalDuration = 0;
  let errorCount = 0;
  let runningCount = 0;
  let successCount = 0;

  for (const tc of tools) {
    typeDistribution[tc.name] = (typeDistribution[tc.name] ?? 0) + 1;
    if (tc.duration) totalDuration += tc.duration;
    if (tc.status === "error") errorCount++;
    else if (tc.status === "running") runningCount++;
    else if (tc.status === "success") successCount++;
  }

  return { typeDistribution, totalDuration, errorCount, runningCount, successCount };
}

function formatDuration(ms: number): string {
  if (ms < 1000) return `${ms}ms`;
  return `${(ms / 1000).toFixed(1)}s`;
}

const FILE_READ_TOOLS = new Set(["file_read", "read_file", "read_skill"]);
const FILE_SEARCH_TOOLS = new Set(["file_search", "list_directory", "list_skills"]);
const SHELL_TOOLS = new Set(["shell", "shell_exec"]);
const WEB_TOOLS = new Set(["web_search", "web_fetch", "http_fetch"]);

function generateSemanticSummary(tools: ToolCall[], t: TFunction<"chat">): string {
  const count = tools.length;
  const names = new Set(tools.map((tc) => tc.name));

  if ([...names].every((n) => FILE_READ_TOOLS.has(n))) return t("stepGroup_readFiles", { count });
  if ([...names].every((n) => FILE_SEARCH_TOOLS.has(n))) return t("stepGroup_searchedLocations", { count });
  if ([...names].every((n) => SHELL_TOOLS.has(n))) return t("stepGroup_executedCommands", { count });
  if ([...names].every((n) => WEB_TOOLS.has(n))) return t("stepGroup_searchedWebPages", { count });

  const allFileOps = [...names].every((n) => FILE_READ_TOOLS.has(n) || FILE_SEARCH_TOOLS.has(n));
  if (allFileOps) return t("stepGroup_exploredFiles", { count });

  return t("stepGroup_executedOps", { count });
}

/**
 * Unified step group — replaces both ToolCallGroupCard and ToolCallGroupTimeline.
 * Collapsed: one-line semantic summary. Expanded: StepIndicator rows.
 */
export const StepGroup = memo(function StepGroup({
  tools,
  streaming = false,
}: {
  tools: ToolCall[];
  streaming?: boolean;
}) {
  const { t } = useTranslation("chat");
  const [expanded, setExpanded] = useState(!streaming);
  const summary = useMemo(() => computeSummary(tools), [tools]);
  const semanticSummary = useMemo(() => generateSemanticSummary(tools, t), [tools, t]);
  const handleToggle = useCallback(() => setExpanded((v) => !v), []);

  const hasErrors = summary.errorCount > 0;
  const allDone = summary.runningCount === 0;

  const visibleTools = useMemo(() => {
    if (expanded) return tools;
    if (streaming && !expanded) return tools.slice(-2);
    return [];
  }, [tools, expanded, streaming]);

  return (
    <div
      style={{
        border: "1px solid var(--step-border)",
        borderRadius: "var(--step-radius)",
        marginBottom: "var(--step-gap)",
        overflow: "hidden",
      }}
    >
      {/* Summary row */}
      <button
        onClick={handleToggle}
        className="flex w-full items-center gap-2 px-2.5 text-left transition-colors duration-100"
        style={{
          cursor: "pointer",
          minHeight: "var(--step-height)",
        }}
        onMouseEnter={(e) => { (e.currentTarget as HTMLElement).style.background = "var(--step-hover-bg)"; }}
        onMouseLeave={(e) => { (e.currentTarget as HTMLElement).style.background = ""; }}
        aria-expanded={expanded}
        aria-label={`${semanticSummary}${expanded ? t("aria_expanded") : t("aria_collapsed")}`}
      >
        {/* Status dot */}
        <span className="flex h-[14px] w-[14px] shrink-0 items-center justify-center">
          {!allDone ? (
            <span
              className="inline-block h-[5px] w-[5px] rounded-full border-[1px]"
              style={{
                borderColor: "var(--tint) transparent transparent transparent",
                animation: "spin 0.8s linear infinite",
              }}
            />
          ) : hasErrors ? (
            <span className="inline-block h-[5px] w-[5px] rounded-full" style={{ background: "var(--red)" }} />
          ) : (
            <span className="inline-block h-[5px] w-[5px] rounded-full" style={{ background: "var(--green)" }} />
          )}
        </span>

        {/* Semantic summary */}
        <span className="flex min-w-0 flex-1 items-center gap-2 text-[12px]">
          <span className="shrink-0 font-medium" style={{ color: "var(--fill-secondary)" }}>
            {semanticSummary}
          </span>
          {hasErrors && (
            <span className="flex items-center gap-0.5 text-[10px]" style={{ color: "var(--red)" }}>
              {t("errorsCount", { count: summary.errorCount })}
            </span>
          )}
        </span>

        {/* Duration */}
        {summary.totalDuration > 0 && (
          <span className="shrink-0 text-[10px] tabular-nums" style={{ color: "var(--fill-quaternary)" }}>
            {formatDuration(summary.totalDuration)}
          </span>
        )}

        <CaretRight
          size={12}
          className="shrink-0 transition-transform duration-150"
          style={{
            color: "var(--fill-quaternary)",
            transform: expanded ? "rotate(90deg)" : "rotate(0)",
          }}
        />
      </button>

      {/* Expanded/streaming visible tools — compact (no borders) inside group */}
      {visibleTools.length > 0 && (
        <div
          className="px-1 pb-1"
          style={{
            borderTop: "1px solid var(--separator)",
          }}
        >
          {!expanded && streaming && tools.length > 2 && (
            <button
              onClick={handleToggle}
              className="py-0.5 px-2 text-[11px] cursor-pointer"
              style={{ color: "var(--fill-quaternary)" }}
            >
              {t("moreCompleted", { count: tools.length - 2 })}
            </button>
          )}
          {visibleTools.map((tc) => (
            <StepIndicator key={tc.id} tool={tc} compact />
          ))}
        </div>
      )}
    </div>
  );
});
