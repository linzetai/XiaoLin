import { useState, useMemo, useCallback, lazy, Suspense } from "react";
import { useTranslation } from "react-i18next";
import {
  Robot, CaretRight, Check, X as XIcon, MagnifyingGlass, Terminal,
  Globe, Wrench, Square, PaperPlaneRight,
} from "@phosphor-icons/react";
import type { SubAgentRunUI, SubAgentToolCall } from "../../lib/stores/types";
import { StepIndicator, extractKeyInfo, type ToolCall } from "./StepIndicator";
import { StreamingMarkdown } from "./StreamingMarkdown";
import { useElapsedTimer, formatElapsed } from "../../lib/hooks/useElapsedTimer";
import * as api from "../../lib/api";

const MarkdownContent = lazy(() =>
  import("./MarkdownContent").then((m) => ({ default: m.MarkdownContent })),
);

/** First non-empty line of a result, with light Markdown markers stripped. */
function resultFirstLine(result: string): string {
  const line = result
    .split("\n")
    .map((s) => s.trim())
    .find((s) => s.length > 0);
  if (!line) return "";
  const cleaned = line
    .replace(/^#+\s*/, "")
    .replace(/^[-*]\s+/, "")
    .replace(/^\d+\.\s+/, "")
    .replace(/^>\s*/, "");
  return cleaned.length > 80 ? cleaned.slice(0, 80) + "…" : cleaned;
}

function useSubAgentCardTypeMeta() {
  const { t } = useTranslation("chat");
  return useMemo(() => {
    const map: Record<string, { icon: React.ReactNode; label: string; color: string }> = {
      general: { icon: <Robot />, label: t("subAgentCard_general"), color: "var(--tint)" },
      explore: { icon: <MagnifyingGlass />, label: t("subAgentCard_explore"), color: "#34c759" },
      shell: { icon: <Terminal />, label: t("subAgentCard_shell"), color: "#ff9500" },
      browser: { icon: <Globe />, label: t("subAgent_browser"), color: "#af52de" },
    };
    return (type: string) => map[type] ?? { icon: <Wrench />, label: type, color: "var(--fill-tertiary)" };
  }, [t]);
}

function adaptToolCall(tc: SubAgentToolCall): ToolCall {
  return {
    id: tc.id,
    name: tc.name,
    status: tc.status as "running" | "success" | "error",
    args: tc.args,
    result: tc.result,
  };
}

interface SubAgentCardProps {
  run: SubAgentRunUI;
  onCancel?: (runId: string) => void;
}

export function SubAgentCard({ run, onCancel }: SubAgentCardProps) {
  const { t } = useTranslation("chat");
  const getTypeMeta = useSubAgentCardTypeMeta();
  const [expanded, setExpanded] = useState(false);
  // Defer mounting the (heavy, Markdown-rendering) expanded body until the card
  // is opened at least once, so collapsed cards stay cheap even in bulk.
  const [hasExpanded, setHasExpanded] = useState(false);
  const [steerInput, setSteerInput] = useState("");
  const [steerSending, setSteerSending] = useState(false);
  const meta = getTypeMeta(run.subagentType);
  const isActive = run.status === "running" || run.status === "pending";
  const isFailed = run.status === "failed" || run.status === "cancelled";
  const elapsed = useElapsedTimer(isActive, run.elapsedMs ?? 0);

  const currentTool = useMemo(() => {
    const running = run.toolCalls.find((tc) => tc.status === "running");
    if (!running) return null;
    const key = extractKeyInfo(adaptToolCall(running));
    return { name: running.name, key };
  }, [run.toolCalls]);

  const toolCount = useMemo(
    () => Math.max(run.toolCallsMade, run.toolCalls.length),
    [run.toolCallsMade, run.toolCalls.length],
  );

  // Collapsed auxiliary line: live tool while running, else a result summary.
  const auxText = useMemo(() => {
    if (isActive && currentTool) {
      return currentTool.key ? `${currentTool.name} · ${currentTool.key}` : currentTool.name;
    }
    if (!isActive && !isFailed && run.result) return resultFirstLine(run.result);
    return null;
  }, [isActive, isFailed, currentTool, run.result]);

  const toggleExpand = useCallback(() => {
    setExpanded((prev) => {
      if (!prev) setHasExpanded(true);
      return !prev;
    });
  }, []);

  const handleSteer = useCallback(async () => {
    const msg = steerInput.trim();
    if (!msg || steerSending) return;
    setSteerSending(true);
    try {
      await api.sendSteeringMessage(run.runId, msg);
      setSteerInput("");
    } catch (e) {
      console.error("Failed to send steering message:", e);
    } finally {
      setSteerSending(false);
    }
  }, [steerInput, steerSending, run.runId]);

  const toolCallsAsSteps = useMemo(
    () => run.toolCalls.map(adaptToolCall),
    [run.toolCalls],
  );

  return (
    <div
      className="rounded"
      style={{
        animation: "fade-slide-up 240ms var(--ease-out, ease-out) both",
        borderLeft: `2px solid ${isFailed ? "var(--red)" : "transparent"}`,
        background: isActive ? "color-mix(in srgb, var(--tint) 4%, transparent)" : undefined,
        transition: "background 200ms ease",
      }}
    >
      {/* Summary row — same visual pattern as StepIndicator. Uses role=button
          instead of <button> so the nested Cancel <button> stays valid HTML. */}
      <div
        role="button"
        tabIndex={0}
        onClick={toggleExpand}
        onKeyDown={(e) => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); toggleExpand(); } }}
        className="flex w-full items-center gap-1.5 py-0.5 text-left transition-colors duration-100 rounded"
        style={{
          cursor: "pointer",
          minHeight: "var(--step-height)",
        }}
        onMouseEnter={(e) => { if (!isActive) (e.currentTarget as HTMLElement).style.background = "var(--step-hover-bg)"; }}
        onMouseLeave={(e) => { if (!isActive) (e.currentTarget as HTMLElement).style.background = ""; }}
        aria-expanded={expanded}
      >
        {/* Status icon */}
        <span className="flex h-[14px] w-[14px] shrink-0 items-center justify-center">
          {isActive ? (
            <span
              className="inline-block h-2.5 w-2.5 rounded-full border-[1.5px]"
              style={{
                borderColor: "var(--tint) transparent transparent transparent",
                animation: "spin 0.8s linear infinite",
              }}
            />
          ) : isFailed ? (
            <XIcon style={{ color: "var(--red)", animation: "scale-spring 240ms var(--ease-out, ease-out) both" }} />
          ) : (
            <Check style={{ color: "var(--green)", animation: "scale-spring 240ms var(--ease-out, ease-out) both" }} />
          )}
        </span>

        {/* Type icon + label + task */}
        <span className="flex min-w-0 flex-1 items-center gap-1.5 text-[12px]">
          <span className="shrink-0" style={{ color: meta.color }}>{meta.icon}</span>
          <span className="shrink-0 font-medium" style={{ color: isFailed ? "var(--red)" : "var(--fill-secondary)" }}>
            {meta.label}
          </span>
          <span
            className="min-w-0 truncate text-[11px]"
            style={{ color: "var(--fill-quaternary)" }}
            title={run.task}
          >
            {run.task.length > 42 ? run.task.slice(0, 42) + "…" : run.task}
          </span>
        </span>

        {/* Duration (live while active) */}
        {isActive && (
          <span className="shrink-0 text-[10px]" style={{ color: "var(--fill-quaternary)" }}>
            {toolCount > 0 ? `已调用 ${toolCount} 个工具` : "进行中"}
          </span>
        )}
        {!isActive && (
          <span className="shrink-0 text-[10px]" style={{ color: isFailed ? "var(--red)" : "var(--fill-quaternary)" }}>
            {isFailed ? "失败" : "已完成"}
          </span>
        )}
        <span className="shrink-0 text-[10px] tabular-nums" style={{ color: "var(--fill-quaternary)" }}>
          {formatElapsed(isActive ? elapsed : run.elapsedMs)}
        </span>

        {isActive && onCancel && (
          <button
            onClick={(e) => { e.stopPropagation(); onCancel(run.runId); }}
            className="flex h-5 w-5 shrink-0 items-center justify-center rounded transition-colors hover:bg-[var(--bg-hover)]"
            title={t("cancel", { ns: "common" })}
            aria-label={t("subAgentCard_cancelAria")}
          >
            <Square style={{ color: "var(--fill-tertiary)" }} />
          </button>
        )}

        <CaretRight
          size={12}
          className="shrink-0 transition-transform duration-150"
          style={{
            color: "var(--fill-quaternary)",
            transform: expanded ? "rotate(90deg)" : "rotate(0)",
          }}
        />
      </div>

      {/* Auxiliary line (collapsed only): current tool while running, else result summary */}
      {!expanded && auxText && (
        <div
          className="flex items-center gap-1 pl-6 pr-2 pb-0.5 text-[10px]"
          style={{ color: isActive ? meta.color : "var(--fill-quaternary)" }}
        >
          {isActive && <span className="shrink-0">▸</span>}
          <span key={auxText} className="min-w-0 truncate" style={{ animation: "fade-in 180ms var(--ease-out, ease-out) both" }} title={auxText}>
            {auxText}
          </span>
        </div>
      )}

      {/* Expanded body — grid-template-rows animation (StepIndicator pattern) */}
      <div
        style={{
          display: "grid",
          gridTemplateRows: expanded ? "1fr" : "0fr",
          transition: "grid-template-rows 260ms cubic-bezier(0.23, 1, 0.32, 1)",
        }}
      >
        <div className="overflow-hidden" inert={!expanded}>
        {hasExpanded && (
        <div
          className="pl-6 pb-1"
          style={{
            borderTop: "1px dashed var(--separator)",
          }}
        >
          {/* Task detail */}
          <div className="mt-1.5 mb-1">
            <span className="text-[10px] font-semibold uppercase tracking-wider" style={{ color: "var(--fill-quaternary)" }}>
              {t("subAgentCard_task")}
            </span>
            <p className="mt-0.5 text-[11px] leading-relaxed" style={{ color: "var(--fill-secondary)" }}>
              {run.task}
            </p>
          </div>

          {/* Streaming content */}
          {run.content && (
            <div
              className="subagent-md mt-1 overflow-auto rounded-md p-2"
              style={{
                background: "var(--bg-primary)",
                border: "0.5px solid var(--separator)",
                maxHeight: "200px",
              }}
            >
              <StreamingMarkdown content={run.content} />
            </div>
          )}

          {/* Tool calls — rendered as StepIndicator rows */}
          {toolCallsAsSteps.length > 0 && (
            <div className="mt-1">
              <span className="text-[10px] font-semibold uppercase tracking-wider" style={{ color: "var(--fill-quaternary)" }}>
                {t("subAgentCard_toolCalls", { count: toolCallsAsSteps.length })}
              </span>
              <div className="mt-0.5">
                {toolCallsAsSteps.map((tc) => (
                  <StepIndicator key={tc.id} tool={tc} compact />
                ))}
              </div>
            </div>
          )}

          {/* Result — Markdown when succeeded, raw <pre> for errors */}
          {run.result && (
            <div className="mt-1.5">
              <span className="text-[10px] font-semibold uppercase tracking-wider" style={{ color: "var(--fill-quaternary)" }}>
                {t("subAgent_result")}
              </span>
              {isFailed ? (
                <pre
                  className="mt-0.5 overflow-x-auto whitespace-pre-wrap break-words rounded-md p-2 text-[11px] leading-[1.55]"
                  style={{
                    background: "var(--bg-primary)",
                    color: "var(--red)",
                    border: "0.5px solid var(--separator)",
                    fontFamily: 'var(--font-mono)',
                    maxHeight: "300px",
                    overflowY: "auto",
                  }}
                >
                  {run.result}
                </pre>
              ) : (
                <div
                  className="subagent-md mt-0.5 overflow-auto rounded-md p-2"
                  style={{
                    background: "var(--bg-primary)",
                    border: "0.5px solid var(--separator)",
                    maxHeight: "300px",
                  }}
                >
                  <Suspense fallback={<div className="animate-pulse rounded py-1" style={{ background: "var(--bg-tertiary)", height: 14 }} />}>
                    <MarkdownContent content={run.result} />
                  </Suspense>
                </div>
              )}
            </div>
          )}

          {/* Notifications */}
          {run.notifications.length > 0 && (
            <div className="mt-1.5">
              <span className="text-[10px] font-semibold uppercase tracking-wider" style={{ color: "var(--fill-quaternary)" }}>
                {t("subAgentCard_notifications")}
              </span>
              <div className="mt-0.5 max-h-[60px] overflow-y-auto space-y-0.5">
                {run.notifications.slice(-3).map((n, i) => (
                  <div key={i} className="text-[10px] leading-tight" style={{ color: "var(--fill-secondary)" }}>
                    <span style={{ color: "var(--fill-quaternary)" }}>
                      {new Date(n.timestamp).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}
                    </span>{" "}
                    {n.message.length > 100 ? n.message.slice(0, 100) + "…" : n.message}
                  </div>
                ))}
              </div>
            </div>
          )}

          {/* Steering input */}
          {isActive && (
            <div className="mt-1.5">
              <div
                className="flex items-center gap-1 rounded-md border px-1.5 py-1"
                style={{ borderColor: "var(--separator)", background: "var(--bg-primary)" }}
              >
                <input
                  type="text"
                  className="flex-1 bg-transparent text-[11px] outline-none"
                  style={{ color: "var(--fill-primary)" }}
                  placeholder={t("subAgentCard_steerPlaceholder")}
                  value={steerInput}
                  onChange={(e) => setSteerInput(e.target.value)}
                  onKeyDown={(e) => { if (e.key === "Enter" && !e.shiftKey) { e.preventDefault(); handleSteer(); } }}
                  disabled={steerSending}
                />
                <button
                  onClick={handleSteer}
                  disabled={!steerInput.trim() || steerSending}
                  className="shrink-0 rounded p-0.5 transition-colors hover:bg-[var(--bg-tertiary)] disabled:opacity-30"
                  title={t("subAgentCard_steerSend")}
                >
                  <PaperPlaneRight size={12} style={{ color: "var(--tint)" }} />
                </button>
              </div>
            </div>
          )}

          {/* Stats */}
          {(run.toolCallsMade > 0 || run.iterations > 0) && (
            <div className="mt-1.5 flex gap-3 text-[10px]" style={{ color: "var(--fill-quaternary)" }}>
              {run.toolCallsMade > 0 && <span>{t("subAgentCard_toolCallsMade", { count: run.toolCallsMade })}</span>}
              {run.iterations > 0 && <span>{t("subAgentCard_iterations", { count: run.iterations })}</span>}
              {run.elapsedMs != null && <span>{t("subAgentCard_elapsed", { seconds: (run.elapsedMs / 1000).toFixed(1) })}</span>}
            </div>
          )}
        </div>
        )}
        </div>
      </div>
    </div>
  );
}
