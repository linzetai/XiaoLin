/**
 * @deprecated Use StepGroup from ./StepGroup.tsx instead.
 * Kept temporarily for backward compatibility during message-stream-redesign transition.
 */
import { useState, useMemo, memo, useCallback } from "react";
import { useTranslation } from "react-i18next";
import {
  CaretRight, Warning, Clock, Stack,
  Check, X as XIcon,
} from "@phosphor-icons/react";
import { ToolCallCard, type ToolCall } from "./ToolCallCard";
import type { StreamSegment } from "./types";
import { isTodoResult } from "./TodoCard";
import { isEditResult } from "./DiffCard";
import { ICON_SIZE } from "../../lib/ui-tokens";

const DEFAULT_THRESHOLD = 3;

export interface ToolCallGroupItem {
  type: "single-tool";
  segment: StreamSegment;
}

export interface ToolCallGroupCluster {
  type: "tool-group";
  segments: StreamSegment[];
}

export interface TextItem {
  type: "text";
  segment: StreamSegment;
}

export type GroupedSegment = ToolCallGroupItem | ToolCallGroupCluster | TextItem;

function isSpecialToolCall(tc: ToolCall): boolean {
  if (!tc.result) return false;
  if (isTodoResult(tc.name, tc.result)) return true;
  if (isEditResult(tc.name, tc.result)) return true;
  return false;
}

/**
 * Groups consecutive tool segments. A group forms when 3+ tool segments
 * appear without text segments in between.
 * DiffCard/TodoCard tools remain individual (not grouped).
 */
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

/**
 * Groups consecutive ToolCall[] from AiMessage.
 */
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

interface ToolGroupSummary {
  typeDistribution: Record<string, number>;
  totalDuration: number;
  errorCount: number;
  runningCount: number;
  successCount: number;
}

function computeSummary(tools: ToolCall[]): ToolGroupSummary {
  const typeDistribution: Record<string, number> = {};
  let maxDuration = 0;
  let errorCount = 0;
  let runningCount = 0;
  let successCount = 0;

  for (const tc of tools) {
    const name = tc.name;
    typeDistribution[name] = (typeDistribution[name] ?? 0) + 1;
    if (tc.duration && tc.duration > maxDuration) maxDuration = tc.duration;
    if (tc.status === "error") errorCount++;
    else if (tc.status === "running") runningCount++;
    else if (tc.status === "success") successCount++;
  }

  return { typeDistribution, totalDuration: maxDuration, errorCount, runningCount, successCount };
}

function formatDuration(ms: number): string {
  if (ms < 1000) return `${ms}ms`;
  return `${(ms / 1000).toFixed(1)}s`;
}

export const ToolCallGroupCard = memo(function ToolCallGroupCard({
  tools,
}: {
  tools: ToolCall[];
}) {
  const { t } = useTranslation("chat");
  const [expanded, setExpanded] = useState(true);
  const summary = useMemo(() => computeSummary(tools), [tools]);

  const topTypes = useMemo(() => {
    const entries = Object.entries(summary.typeDistribution);
    entries.sort((a, b) => b[1] - a[1]);
    return entries.slice(0, 3);
  }, [summary.typeDistribution]);

  const handleToggle = useCallback(() => setExpanded((v) => !v), []);

  const hasErrors = summary.errorCount > 0;
  const allDone = summary.runningCount === 0;

  return (
    <div
      className="my-1.5 overflow-hidden rounded-lg"
      style={{
        border: `0.5px solid ${hasErrors ? "color-mix(in srgb, var(--red) 30%, transparent)" : "var(--separator)"}`,
        background: "var(--bg-secondary)",
        maxWidth: "min(100%, 600px)",
      }}
    >
      <button
        onClick={handleToggle}
        className="flex w-full items-center gap-2 px-3 py-2 text-left transition-all duration-150 hover:brightness-[1.04]"
        style={{ cursor: "pointer" }}
        aria-expanded={expanded}
        aria-label={`${t("toolCallsCount", { count: tools.length })}${expanded ? t("aria_expanded") : t("aria_collapsed")}`}
      >
        <span className="flex h-4 w-4 shrink-0 items-center justify-center">
          <Stack size={ICON_SIZE.md} style={{ color: "var(--fill-tertiary)" }} />
        </span>

        <span className="flex min-w-0 flex-1 items-center gap-2 overflow-hidden text-[12px]">
          <span className="shrink-0 font-medium" style={{ color: "var(--fill-primary)" }}>
            {t("toolCallsCount", { count: tools.length })}
          </span>
          <span className="flex min-w-0 flex-1 items-center gap-1.5 overflow-hidden text-[11px]" style={{ color: "var(--fill-tertiary)" }}>
            {topTypes.map(([name, count]) => (
              <span key={name} className="inline-flex shrink-0 items-center gap-0.5 rounded px-1 py-0.5" style={{ background: "var(--bg-primary)" }}>
                {name.replace(/_/g, " ")}
                {count > 1 && <span className="text-[10px] tabular-nums">×{count}</span>}
              </span>
            ))}
          </span>
        </span>

        <span className="flex shrink-0 items-center gap-2">
          {hasErrors && (
            <span className="flex items-center gap-0.5 text-[10px]" style={{ color: "var(--red)" }}>
              <Warning />
              {summary.errorCount}
            </span>
          )}
          {!hasErrors && allDone && (
            <span className="flex items-center gap-0.5 text-[10px]" style={{ color: "var(--fill-quaternary)" }}>
              <Check  />
              {summary.successCount}
            </span>
          )}
          {!allDone && (
            <span
              className="inline-block h-3 w-3 rounded-full border-[1.5px]"
              style={{
                borderColor: "var(--fill-tertiary) transparent transparent transparent",
                animation: "spin 0.8s linear infinite",
              }}
            />
          )}
          {summary.totalDuration > 0 && (
            <span className="flex items-center gap-0.5 text-[10px] tabular-nums" style={{ color: "var(--fill-quaternary)" }}>
              <Clock  />
              {formatDuration(summary.totalDuration)}
            </span>
          )}
          <CaretRight
            className="shrink-0 transition-transform duration-150"
            style={{
              color: "var(--fill-quaternary)",
              transform: expanded ? "rotate(90deg)" : "rotate(0)",
            }}
          />
        </span>
      </button>

      {!expanded && hasErrors && (
        <div className="px-3 pb-2">
          {tools
            .filter((tc) => tc.status === "error")
            .map((tc) => (
              <ToolCallCard key={tc.id} tool={tc} />
            ))}
        </div>
      )}

      {expanded && (
        <div
          className="px-3 pb-2"
          style={{
            borderTop: "0.5px solid var(--separator)",
          }}
        >
          {tools.map((tc) => (
            <ToolCallCard key={tc.id} tool={tc} />
          ))}
        </div>
      )}
    </div>
  );
});

/**
 * Streaming-mode compact timeline for tool call groups.
 * Each row is ~28px vs normal ~44px.
 */
export const ToolCallGroupTimeline = memo(function ToolCallGroupTimeline({
  tools,
}: {
  tools: ToolCall[];
}) {
  const { t } = useTranslation("chat");
  const [expanded, setExpanded] = useState(false);
  const summary = useMemo(() => computeSummary(tools), [tools]);
  const handleToggle = useCallback(() => setExpanded((v) => !v), []);

  const { visibleTools, hiddenCount } = useMemo(() => {
    if (expanded) {
      return { visibleTools: tools, hiddenCount: tools.length - 2 - tools.filter((tc) => tc.status === "error").length };
    }
    const last2 = tools.slice(-2);
    const errors = tools.filter((tc) => tc.status === "error" && !last2.includes(tc));
    return { visibleTools: [...errors, ...last2], hiddenCount: tools.length - errors.length - 2 };
  }, [tools, expanded]);

  // 始终显示按钮，只要有多于2个工具或有错误
  const showToggle = tools.length > 2 || summary.errorCount > 0;

  return (
    <div
      className="my-1.5 overflow-hidden rounded-lg"
      style={{
        border: "0.5px solid var(--separator)",
        background: "var(--bg-secondary)",
        maxWidth: "min(100%, 600px)",
      }}
    >
      {showToggle && (
        <button
          onClick={handleToggle}
          className="flex w-full items-center gap-2 px-3 py-1 text-left transition-colors duration-100"
          style={{ cursor: "pointer", borderBottom: "0.5px solid var(--separator)" }}
          aria-expanded={expanded}
        >
          <Stack style={{ color: "var(--fill-quaternary)" }} />
          <span className="text-[11px]" style={{ color: "var(--fill-tertiary)" }}>
            {expanded ? t("collapse", { ns: "common" }) : t("moreCompleted", { count: hiddenCount })}
          </span>
          {summary.errorCount > 0 && (
            <span className="flex items-center gap-0.5 text-[10px]" style={{ color: "var(--red)" }}>
              <XIcon  />
              {summary.errorCount}
            </span>
          )}
          <CaretRight
            className="ml-auto shrink-0 transition-transform duration-150"
            style={{
              color: "var(--fill-quaternary)",
              transform: expanded ? "rotate(90deg)" : "rotate(0)",
            }}
          />
        </button>
      )}

      <div className="relative pl-5">
        <div
          className="absolute left-[14px] top-2 bottom-2 w-[1.5px]"
          style={{ background: "var(--separator)" }}
        />
        {visibleTools.map((tc) => (
          <TimelineRow key={tc.id} tool={tc} />
        ))}
      </div>
    </div>
  );
});

const TimelineRow = memo(function TimelineRow({ tool }: { tool: ToolCall }) {
  const [expanded, setExpanded] = useState(false);
  const isRunning = tool.status === "running";
  const isError = tool.status === "error";
  const hasDetails = !!(tool.args || tool.result);
  const handleToggle = useCallback(() => {
    if (hasDetails) setExpanded((v) => !v);
  }, [hasDetails]);
  const keyInfo = useMemo(() => {
    if (!tool.args) return null;
    try {
      const args = JSON.parse(tool.args);
      return args.path ?? args.file ?? args.command ?? args.cmd ?? args.query ?? args.url ?? null;
    } catch {
      return null;
    }
  }, [tool.args]);

  return (
    <div>
      <div
        className="flex items-center gap-2 py-[3px] text-[11px]"
        style={{ cursor: hasDetails ? "pointer" : "default" }}
        onClick={handleToggle}
        role={hasDetails ? "button" : undefined}
        aria-expanded={hasDetails ? expanded : undefined}
      >
        <span className="relative z-10 flex shrink-0 items-center justify-center rounded-full"
          style={{
            width: isRunning ? 9 : 7,
            height: isRunning ? 9 : 7,
            background: isError ? "var(--red)" : isRunning ? "var(--tint)" : "var(--fill-quaternary)",
            boxShadow: isRunning ? "0 0 0 2px var(--bg-secondary), 0 0 0 3.5px color-mix(in srgb, var(--tint) 30%, transparent)" : undefined,
            animation: isRunning ? "pulse 1.5s ease-in-out infinite" : undefined,
          }}
        />
        {isRunning && (
          <span
            className="inline-block h-3 w-3 shrink-0 rounded-full border-[1.5px]"
            style={{
              borderColor: "var(--tint) transparent transparent transparent",
              animation: "spin 0.8s linear infinite",
            }}
          />
        )}
        <span className="shrink-0 font-medium" style={{ color: isError ? "var(--red)" : isRunning ? "var(--tint)" : "var(--fill-secondary)" }}>
          {tool.name.replace(/_/g, " ")}
        </span>
        {keyInfo && (
          <span className="min-w-0 flex-1 truncate font-mono text-[10px]" style={{ color: "var(--fill-quaternary)" }} title={keyInfo}>
            {keyInfo}
          </span>
        )}
        <span className="ml-auto shrink-0 tabular-nums text-[10px]" style={{ color: isRunning ? "var(--tint)" : "var(--fill-quaternary)", minWidth: "3em" }}>
          {isRunning && tool.startTime && `${((Date.now() - tool.startTime) / 1000).toFixed(1)}s`}
          {!isRunning && tool.duration ? formatDuration(tool.duration) : null}
        </span>
        {hasDetails && (
          <CaretRight
            className="shrink-0 transition-transform duration-150"
            style={{
              color: "var(--fill-quaternary)",
              transform: expanded ? "rotate(90deg)" : "rotate(0)",
            }}
          />
        )}
      </div>
      {expanded && hasDetails && (
        <div className="ml-4 mb-1">
          <ToolCallCard tool={tool} />
        </div>
      )}
    </div>
  );
});
