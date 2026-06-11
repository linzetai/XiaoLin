import { useState, useMemo } from "react";
import { useTranslation } from "react-i18next";
import {
  Robot, CaretRight, Check, X as XIcon, MagnifyingGlass, Terminal,
  Globe, Wrench, Square,
} from "@phosphor-icons/react";
import type { SubAgentRunUI, SubAgentToolCall } from "../../lib/agent-store";
import { StepIndicator, type ToolCall } from "./StepIndicator";

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
  const meta = getTypeMeta(run.subagentType);
  const isActive = run.status === "running" || run.status === "pending";
  const isFailed = run.status === "failed" || run.status === "cancelled";

  const toolCallsAsSteps = useMemo(
    () => run.toolCalls.map(adaptToolCall),
    [run.toolCalls],
  );

  return (
    <div>
      {/* Summary row — same visual pattern as StepIndicator */}
      <button
        onClick={() => setExpanded(!expanded)}
        className="flex w-full items-center gap-1.5 py-0.5 text-left transition-colors duration-100 rounded"
        style={{
          cursor: "pointer",
          minHeight: "var(--step-height)",
          background: isActive ? "color-mix(in srgb, var(--tint) 4%, transparent)" : undefined,
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
            <XIcon style={{ color: "var(--red)" }} />
          ) : (
            <Check style={{ color: "var(--green)" }} />
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
            {run.task.length > 60 ? run.task.slice(0, 60) + "…" : run.task}
          </span>
        </span>

        {/* Duration */}
        {run.elapsedMs != null && (
          <span className="shrink-0 text-[10px] tabular-nums" style={{ color: "var(--fill-quaternary)" }}>
            {run.elapsedMs < 1000 ? `${run.elapsedMs}ms` : `${(run.elapsedMs / 1000).toFixed(1)}s`}
          </span>
        )}

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
      </button>

      {/* Expanded: task + tool calls as StepIndicator rows */}
      {expanded && (
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
            <div className="mt-1">
              <pre
                className="overflow-x-auto whitespace-pre-wrap break-words rounded-md p-2 text-[11px] leading-[1.55]"
                style={{
                  background: "var(--bg-primary)",
                  color: "var(--fill-secondary)",
                  border: "0.5px solid var(--separator)",
                  fontFamily: 'var(--font-mono)',
                  maxHeight: "200px",
                  overflowY: "auto",
                }}
              >
                {run.content}
              </pre>
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

          {/* Result */}
          {run.result && (
            <div className="mt-1.5">
              <span className="text-[10px] font-semibold uppercase tracking-wider" style={{ color: "var(--fill-quaternary)" }}>
                {t("subAgent_result")}
              </span>
              <pre
                className="mt-0.5 overflow-x-auto whitespace-pre-wrap break-words rounded-md p-2 text-[11px] leading-[1.55]"
                style={{
                  background: "var(--bg-primary)",
                  color: isFailed ? "var(--red)" : "var(--fill-secondary)",
                  border: "0.5px solid var(--separator)",
                  fontFamily: 'var(--font-mono)',
                  maxHeight: "300px",
                  overflowY: "auto",
                }}
              >
                {run.result}
              </pre>
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
  );
}
