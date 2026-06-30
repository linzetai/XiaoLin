// ToolStepView — renders a ToolStepNode from the canonical timeline.
//
// Handles all tool step states per spec:
// - Compact header: status dot + icon + display_title + key_info + duration
// - Progress bar when running (with progress_label)
// - Small output inline preview (6.2)
// - Large output lazy expansion via handle-based API (6.3)
// - Paged/sectional detail UI (6.4)
// - Structured renderers by content_type (6.5)

import { useState, useCallback, useMemo, memo, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import {
  FileText, MagnifyingGlass, Terminal, Globe,
  Brain, Database, Wrench, Check,
  CaretRight, Plug, ListChecks,
} from "@phosphor-icons/react";
import type {
  ToolStepNode,
  OutputPreview,
} from "../../lib/timeline/types";
import { SMALL_OUTPUT_MAX_BYTES, SMALL_OUTPUT_MAX_LINES, SMALL_OUTPUT_MAX_TOKENS } from "../../lib/timeline/types";
import * as api from "../../lib/api";
import { ICON_SIZE } from "../../lib/ui-tokens";

// ============================================================================
// Icons by tool category
// ============================================================================

const CATEGORY_ICONS: Record<string, ReactNode> = {
  file: <FileText size={ICON_SIZE.sm} />,
  shell: <Terminal size={ICON_SIZE.sm} />,
  search: <MagnifyingGlass size={ICON_SIZE.sm} />,
  web: <Globe size={ICON_SIZE.sm} />,
  mcp: <Plug size={ICON_SIZE.sm} />,
  interaction: <Check size={ICON_SIZE.sm} />,
  sub_agent: <Brain size={ICON_SIZE.sm} />,
  memory: <Database size={ICON_SIZE.sm} />,
  planning: <ListChecks size={ICON_SIZE.sm} />,
  other: <Wrench size={ICON_SIZE.sm} />,
};

function iconForCategory(cat?: string): ReactNode {
  return CATEGORY_ICONS[cat ?? ""] ?? CATEGORY_ICONS.other;
}

// ============================================================================
// Helpers
// ============================================================================

function formatDuration(ms: number): string {
  if (ms < 1000) return `${ms}ms`;
  return `${(ms / 1000).toFixed(1)}s`;
}

function tryPrettyJson(text: string): string {
  try {
    return JSON.stringify(JSON.parse(text), null, 2);
  } catch {
    return text;
  }
}

function extractKeyInfo(node: ToolStepNode): string | null {
  const t = node.target;
  if (!t) return null;
  return t.path ?? t.command ?? t.url ?? t.query ?? t.label ?? null;
}

// ============================================================================
// Small output policy check (6.2)
// ============================================================================

function isSmallOutput(preview: OutputPreview): boolean {
  return (
    !preview.is_binary &&
    preview.byte_length <= SMALL_OUTPUT_MAX_BYTES &&
    preview.line_count <= SMALL_OUTPUT_MAX_LINES &&
    preview.estimated_tokens <= SMALL_OUTPUT_MAX_TOKENS
  );
}

// ============================================================================
// Structured renderers by content_type (6.5)
// ============================================================================

function CommandOutputView({ content }: { content: string }) {
  // Show command output with monospace, no syntax highlighting needed
  return (
    <pre
      className="overflow-x-auto whitespace-pre-wrap break-all rounded-md p-2 text-[11px] leading-[1.55]"
      style={{
        background: "var(--bg-primary)",
        color: "var(--fill-secondary)",
        border: "0.5px solid var(--separator)",
        fontFamily: "var(--font-mono)",
        maxHeight: 280,
        overflowY: "auto",
      }}
    >
      {content}
    </pre>
  );
}

function SearchResultsView({ content }: { content: string }) {
  const lines = content.split("\n").filter(Boolean);
  return (
    <div className="space-y-0.5">
      {lines.slice(0, 20).map((line, i) => (
        <div
          key={i}
          className="truncate rounded px-1.5 py-0.5 text-[11px]"
          style={{
            background: "var(--bg-primary)",
            color: "var(--fill-secondary)",
            fontFamily: "var(--font-mono)",
          }}
          title={line}
        >
          {line}
        </div>
      ))}
      {lines.length > 20 && (
        <div className="text-[10px] px-1.5" style={{ color: "var(--fill-quaternary)" }}>
          +{lines.length - 20} more results
        </div>
      )}
    </div>
  );
}

function JsonView({ content }: { content: string }) {
  const formatted = useMemo(() => tryPrettyJson(content), [content]);
  return (
    <pre
      className="overflow-x-auto rounded-md p-2 text-[11px] leading-[1.55]"
      style={{
        background: "var(--bg-primary)",
        color: "var(--fill-secondary)",
        border: "0.5px solid var(--separator)",
        fontFamily: "var(--font-mono)",
        maxHeight: 280,
        overflowY: "auto",
      }}
    >
      {formatted}
    </pre>
  );
}

function ErrorOutputView({ content }: { content: string }) {
  return (
    <pre
      className="overflow-x-auto whitespace-pre-wrap break-all rounded-md p-2 text-[11px] leading-[1.55]"
      style={{
        background: "color-mix(in srgb, var(--red) 4%, transparent)",
        color: "var(--red)",
        border: "0.5px solid color-mix(in srgb, var(--red) 15%, transparent)",
        fontFamily: "var(--font-mono)",
        maxHeight: 200,
        overflowY: "auto",
      }}
    >
      {content.slice(0, 2000)}
    </pre>
  );
}

function TextOutputView({ content }: { content: string }) {
  const lines = content.split("\n");
  const truncated = lines.length > 200 || content.length > 4000;
  const display = truncated
    ? lines.slice(0, 200).join("\n").slice(0, 4000)
    : content;

  return (
    <pre
      className="overflow-x-auto whitespace-pre-wrap break-all rounded-md p-2 text-[11px] leading-[1.55]"
      style={{
        background: "var(--bg-primary)",
        color: "var(--fill-secondary)",
        border: "0.5px solid var(--separator)",
        fontFamily: "var(--font-mono)",
        maxHeight: 280,
        overflowY: "auto",
      }}
    >
      {display}
      {truncated && (
        <span style={{ color: "var(--fill-quaternary)" }}>…</span>
      )}
    </pre>
  );
}

function StructuredOutput({ content, contentType }: { content: string; contentType?: string }) {
  switch (contentType) {
    case "command_output":
      return <CommandOutputView content={content} />;
    case "search_results":
      return <SearchResultsView content={content} />;
    case "json":
      return <JsonView content={content} />;
    case "error":
      return <ErrorOutputView content={content} />;
    default:
      return <TextOutputView content={content} />;
  }
}

// ============================================================================
// Detail panel — handles large output expansion (6.3, 6.4)
// ============================================================================

type DetailMode = "head" | "range" | "tail";

interface DetailState {
  content: string;
  truncated: boolean;
  totalBytes: number;
  totalLines: number;
  nextOffset?: number;
  mode: DetailMode;
  rangeStart?: number;
  rangeEnd?: number;
  tailLines?: number;
}

function ToolDetailPanel({
  sessionId,
  handle,
  byteLength,
  lineCount,
  summary,
  contentType,
}: {
  sessionId: string;
  handle: string;
  byteLength: number;
  lineCount: number;
  summary?: string;
  contentType?: string;
}) {
  const { t } = useTranslation("chat");
  const [detail, setDetail] = useState<DetailState | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const loadHead = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const resp = await api.getToolOutputDetail(sessionId, handle);
      setDetail({
        content: (resp.content as string) ?? "",
        truncated: (resp.truncated as boolean) ?? false,
        totalBytes: (resp.total_bytes as number) ?? byteLength,
        totalLines: (resp.total_lines as number) ?? lineCount,
        nextOffset: (resp.continuation as Record<string, unknown>)?.next_offset as number | undefined,
        mode: "head",
      });
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }, [sessionId, handle, byteLength, lineCount]);

  const loadTail = useCallback(async (lines: number) => {
    setLoading(true);
    setError(null);
    try {
      const resp = await api.getToolOutputDetail(sessionId, handle, { tail_lines: lines });
      setDetail({
        content: (resp.content as string) ?? "",
        truncated: (resp.truncated as boolean) ?? false,
        totalBytes: (resp.total_bytes as number) ?? byteLength,
        totalLines: (resp.total_lines as number) ?? lineCount,
        tailLines: lines,
        mode: "tail",
      });
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }, [sessionId, handle, byteLength, lineCount]);

  const loadRange = useCallback(async (start: number, end: number) => {
    setLoading(true);
    setError(null);
    try {
      const resp = await api.getToolOutputDetail(sessionId, handle, { range_start: start, range_end: end });
      setDetail({
        content: (resp.content as string) ?? "",
        truncated: (resp.truncated as boolean) ?? false,
        totalBytes: (resp.total_bytes as number) ?? byteLength,
        totalLines: (resp.total_lines as number) ?? lineCount,
        rangeStart: start,
        rangeEnd: end,
        mode: "range",
      });
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }, [sessionId, handle, byteLength, lineCount]);

  // Auto-load head on first expand
  if (!detail && !loading && !error) {
    return (
      <div className="pl-6 pb-2 pt-0.5">
        <div className="mb-1.5">
          <span className="text-[10px] font-semibold uppercase tracking-wider" style={{ color: "var(--fill-quaternary)" }}>
            {t("outputDetail")}
          </span>
          <div className="mt-1 text-[11px]" style={{ color: "var(--fill-tertiary)" }}>
            {summary ?? `${byteLength.toLocaleString()} bytes, ${lineCount} lines`}
          </div>
        </div>
        <div className="flex gap-2">
          <button
            onClick={loadHead}
            disabled={loading}
            className="cursor-pointer rounded px-2.5 py-1 text-[11px] font-medium transition-colors"
            style={{ background: "var(--tint)", color: "var(--bg-primary)" }}
          >
            {t("loadOutput")}
          </button>
          <button
            onClick={() => loadTail(100)}
            disabled={loading}
            className="cursor-pointer rounded px-2.5 py-1 text-[11px] font-medium transition-colors"
            style={{ background: "var(--bg-secondary)", color: "var(--fill-secondary)", border: "0.5px solid var(--separator)" }}
          >
            {t("viewTail")}
          </button>
        </div>
      </div>
    );
  }

  // Loading state
  if (loading) {
    return (
      <div className="pl-6 pb-2 pt-0.5">
        <div className="animate-pulse rounded py-2 text-[11px]" style={{ background: "var(--bg-tertiary)", color: "var(--fill-tertiary)", height: 24 }}>
          {t("loadingFullOutput")}
        </div>
      </div>
    );
  }

  // Error state
  if (error) {
    return (
      <div className="pl-6 pb-2 pt-0.5">
        <div className="rounded-md p-2 text-[11px]" style={{ background: "color-mix(in srgb, var(--red) 4%, transparent)", color: "var(--red)" }}>
          {error}
        </div>
      </div>
    );
  }

  // Empty state
  if (!detail) return null;

  // Rendered detail with pagination controls
  return (
    <div className="pl-6 pb-2 pt-0.5">
      {/* Metadata */}
      <div className="mb-1 flex items-center gap-2 text-[10px]" style={{ color: "var(--fill-quaternary)" }}>
        <span>{detail.totalBytes.toLocaleString()} bytes</span>
        <span>·</span>
        <span>{detail.totalLines} lines</span>
        {detail.mode === "head" && detail.truncated && (
          <span>· <span style={{ color: "var(--amber)" }}>{t("truncated")}</span></span>
        )}
      </div>

      {/* Content */}
      <StructuredOutput content={detail.content} contentType={contentType} />

      {/* Pagination controls (6.4) */}
      {detail.truncated && detail.mode === "head" && detail.nextOffset != null && (
        <button
          onClick={() => loadRange(detail.nextOffset!, detail.nextOffset! + 65536)}
          className="mt-1 cursor-pointer text-[11px] font-medium"
          style={{ color: "var(--tint)" }}
        >
          {t("loadMore")} →
        </button>
      )}

      {detail.mode === "tail" && detail.truncated && (
        <button
          onClick={() => loadTail((detail.tailLines ?? 100) + 200)}
          className="mt-1 cursor-pointer text-[11px] font-medium"
          style={{ color: "var(--tint)" }}
        >
          {t("loadMoreLines")} →
        </button>
      )}

      {detail.mode === "range" && detail.truncated && detail.rangeEnd != null && (
        <button
          onClick={() => loadRange(detail.rangeEnd!, detail.rangeEnd! + 65536)}
          className="mt-1 cursor-pointer text-[11px] font-medium"
          style={{ color: "var(--tint)" }}
        >
          {t("loadMore")} →
        </button>
      )}

      {/* Reload options */}
      <div className="mt-1.5 flex gap-1.5">
        <button
          onClick={loadHead}
          className="cursor-pointer rounded px-2 py-0.5 text-[10px]"
          style={{ background: "var(--bg-secondary)", color: "var(--fill-tertiary)", border: "0.5px solid var(--separator)" }}
        >
          {t("viewHead")}
        </button>
        <button
          onClick={() => loadTail(100)}
          className="cursor-pointer rounded px-2 py-0.5 text-[10px]"
          style={{ background: "var(--bg-secondary)", color: "var(--fill-tertiary)", border: "0.5px solid var(--separator)" }}
        >
          {t("viewTail")}
        </button>
      </div>
    </div>
  );
}

// ============================================================================
// ToolStepView — main component
// ============================================================================

export interface ToolStepViewProps {
  node: ToolStepNode;
  sessionId?: string;
  presentationTitle?: string;
}

export const ToolStepView = memo(function ToolStepView({
  node,
  sessionId,
  presentationTitle,
}: ToolStepViewProps) {
  const { t } = useTranslation("chat");
  const [expanded, setExpanded] = useState(false);

  const isRunning = node.status === "running";
  const isFailed = node.status === "failed";
  const isCancelled = node.status === "cancelled";
  const isError = isFailed || isCancelled;
  const isCompleted = node.status === "completed";

  const keyInfo = useMemo(() => extractKeyInfo(node), [node.target]);
  const canExpand = !!(
    node.args ||
    node.output_preview ||
    node.output_detail ||
    node.error_message
  );

  const handleToggle = useCallback(() => {
    if (canExpand) setExpanded((v) => !v);
  }, [canExpand]);

  const hasSmallOutput =
    isCompleted && node.output_preview && isSmallOutput(node.output_preview);
  const hasLargeOutput = isCompleted && !!node.output_detail;
  const hasError = isError && !!node.error_message;

  return (
    <div
      className="timeline-tool-step"
      data-activity-row
      style={{
        margin: "2px 0",
        marginBottom: expanded ? 6 : 2,
        overflow: "hidden",
      }}
    >
      <button
        onClick={handleToggle}
        className="group flex w-full items-center gap-2 rounded-md px-1.5 py-1 text-left transition-colors duration-100"
        style={{
          cursor: canExpand ? "pointer" : "default",
          minHeight: 26,
          color: "var(--fill-tertiary)",
        }}
        onMouseEnter={(e) => {
          if (canExpand) {
            (e.currentTarget as HTMLElement).style.background = "var(--bg-secondary)";
          }
        }}
        onMouseLeave={(e) => {
          (e.currentTarget as HTMLElement).style.background = "";
        }}
        aria-expanded={canExpand ? expanded : undefined}
      >
        {canExpand && (
          <CaretRight
            size={11}
            className="shrink-0 transition-transform duration-150"
            style={{
              color: "var(--fill-quaternary)",
              transform: expanded ? "rotate(90deg)" : "rotate(0)",
            }}
          />
        )}
        {!canExpand && <span className="w-[11px] shrink-0" />}

        <span className="flex h-[16px] w-[16px] shrink-0 items-center justify-center">
          {isRunning ? (
            <span
              className="inline-block h-[7px] w-[7px] rounded-full"
              style={{
                borderWidth: "1.5px",
                borderStyle: "solid",
                borderColor: "var(--tint) transparent transparent transparent",
                animation: "spin 0.8s linear infinite",
              }}
            />
          ) : isError ? (
            <span
              className="inline-block h-[7px] w-[7px] rounded-full"
              style={{ background: "var(--red)" }}
            />
          ) : (
            <span
              className="inline-block h-[7px] w-[7px] rounded-full"
              style={{ background: "var(--green)" }}
            />
          )}
        </span>

        <span className="shrink-0" style={{ color: "var(--fill-quaternary)", opacity: 0.85 }}>
          {iconForCategory(node.tool_category)}
        </span>

        <span className="flex min-w-0 flex-1 items-baseline gap-2 text-[12px] leading-5">
          <span
            className="min-w-0 truncate font-medium"
            style={{ color: isError ? "var(--red)" : "var(--fill-secondary)" }}
          >
            {presentationTitle ?? node.display_title}
          </span>
          {keyInfo && (
            <span
              className="min-w-0 truncate text-[11px]"
              style={{
                color: "var(--fill-quaternary)",
                fontFamily: "var(--font-mono)",
              }}
              title={keyInfo}
            >
              {keyInfo}
            </span>
          )}
        </span>

        <span className="flex shrink-0 items-center gap-1">
          <span
            className="text-[10px] tabular-nums"
            style={{ color: "var(--fill-quaternary)" }}
          >
            {isRunning && node.started_at_ms
              ? `${((Date.now() - node.started_at_ms) / 1000).toFixed(1)}s`
              : node.duration_ms != null
                ? formatDuration(node.duration_ms)
              : null}
          </span>
        </span>
      </button>

      {/* Progress bar */}
      {isRunning && node.progress != null && (
        <div className="ml-[54px] flex items-center gap-2 pb-1">
          <div
            className="flex-1 h-[3px] rounded-full overflow-hidden"
            style={{ background: "var(--bg-tertiary)" }}
          >
            <div
              className="h-full rounded-full transition-all duration-300"
              style={{
                width: `${Math.min(100, Math.max(0, node.progress * 100))}%`,
                background: "var(--tint)",
              }}
            />
          </div>
          {node.progress_label && (
            <span
              className="text-[10px] truncate"
              style={{ color: "var(--fill-quaternary)", maxWidth: "60%" }}
            >
              {node.progress_label}
            </span>
          )}
        </div>
      )}

      {/* Expanded detail area */}
      <div
        style={{
          display: "grid",
          gridTemplateRows: expanded && canExpand ? "1fr" : "0fr",
          transition: "grid-template-rows 260ms cubic-bezier(0.23, 1, 0.32, 1)",
        }}
      >
        <div className="overflow-hidden">
          <div className="ml-[54px] border-l pb-2 pl-3 pt-1" style={{ borderColor: "var(--separator)" }}>
            {/* Args */}
            {node.args && (
              <div className="mb-1.5">
                <span
                  className="text-[10px] font-semibold uppercase tracking-wider"
                  style={{ color: "var(--fill-quaternary)" }}
                >
                  {t("params")}
                </span>
                <pre
                  className="mt-0.5 overflow-x-auto whitespace-pre-wrap break-all rounded-md p-2 text-[11px] leading-[1.5]"
                  style={{
                    background: "var(--bg-primary)",
                    color: "var(--fill-secondary)",
                    border: "0.5px solid var(--separator)",
                    fontFamily: "var(--font-mono)",
                    maxHeight: 200,
                    overflowY: "auto",
                  }}
                >
                  {tryPrettyJson(node.args)}
                </pre>
              </div>
            )}

            {/* Small output inline (6.2) */}
            {hasSmallOutput && node.output_preview && (
              <div className="mb-1.5">
                <span
                  className="text-[10px] font-semibold uppercase tracking-wider"
                  style={{ color: "var(--fill-quaternary)" }}
                >
                  {t("output")}
                </span>
                <div className="mt-0.5">
                  <StructuredOutput
                    content={node.output_preview.content}
                    contentType={node.output_preview.content_type}
                  />
                </div>
                <div
                  className="mt-0.5 text-[10px]"
                  style={{ color: "var(--fill-quaternary)" }}
                >
                  {node.output_preview.byte_length.toLocaleString()} bytes ·{" "}
                  {node.output_preview.line_count} lines
                </div>
              </div>
            )}

            {/* Large output detail reference (6.3, 6.4) */}
            {hasLargeOutput &&
              node.output_detail &&
              sessionId && (
                <ToolDetailPanel
                  sessionId={sessionId}
                  handle={node.output_detail.handle}
                  byteLength={node.output_detail.byte_length}
                  lineCount={node.output_detail.line_count}
                  summary={node.output_detail.summary}
                  contentType={node.output_detail.content_type}
                />
              )}

            {/* Error message */}
            {hasError && node.error_message && (
              <div className="mb-1.5">
                <span
                  className="text-[10px] font-semibold uppercase tracking-wider"
                  style={{ color: "var(--red)" }}
                >
                  {t("error")}
                </span>
                <StructuredOutput
                  content={node.error_message}
                  contentType="error"
                />
              </div>
            )}
          </div>
        </div>
      </div>
    </div>
  );
});
