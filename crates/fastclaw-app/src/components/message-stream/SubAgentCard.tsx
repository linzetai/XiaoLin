import { useState } from "react";
import {
  Bot, ChevronRight, Check, X as XIcon, Loader, Search, Terminal,
  Globe, Wrench, Square,
} from "lucide-react";
import type { SubAgentRunUI, SubAgentToolCall } from "../../lib/agent-store";

const ICON_PROPS = { size: 13, strokeWidth: 1.5 } as const;

const TYPE_META: Record<string, { icon: React.ReactNode; label: string; color: string }> = {
  general: { icon: <Bot {...ICON_PROPS} />, label: "通用子智能体", color: "var(--tint)" },
  explore: { icon: <Search {...ICON_PROPS} />, label: "探索 (只读)", color: "#34c759" },
  shell: { icon: <Terminal {...ICON_PROPS} />, label: "命令执行", color: "#ff9500" },
  browser: { icon: <Globe {...ICON_PROPS} />, label: "浏览器", color: "#af52de" },
};

function getTypeMeta(type: string) {
  return TYPE_META[type] ?? { icon: <Wrench {...ICON_PROPS} />, label: type, color: "var(--fill-tertiary)" };
}

function StatusIndicator({ status }: { status: SubAgentRunUI["status"] }) {
  switch (status) {
    case "pending":
    case "running":
      return (
        <span
          className="inline-block h-3 w-3 rounded-full border-[1.5px]"
          style={{
            borderColor: "var(--tint) transparent transparent transparent",
            animation: "spin 0.8s linear infinite",
          }}
        />
      );
    case "completed":
      return <Check size={12} strokeWidth={2} style={{ color: "var(--green, #34c759)" }} />;
    case "failed":
      return <XIcon size={12} strokeWidth={2} style={{ color: "var(--red)" }} />;
    case "cancelled":
      return <Square size={10} strokeWidth={2} style={{ color: "var(--fill-quaternary)" }} />;
  }
}

function MiniToolCall({ tc }: { tc: SubAgentToolCall }) {
  const isRunning = tc.status === "running";
  const isError = tc.status === "error";
  return (
    <div
      className="flex items-center gap-1.5 rounded px-1.5 py-0.5 text-[10.5px]"
      style={{
        background: "var(--bg-primary)",
        border: "0.5px solid var(--separator)",
        color: isError ? "var(--red)" : "var(--fill-secondary)",
      }}
    >
      <span className="flex h-3 w-3 items-center justify-center">
        {isRunning ? (
          <Loader size={9} className="animate-spin" style={{ color: "var(--fill-tertiary)" }} />
        ) : isError ? (
          <XIcon size={9} strokeWidth={2} style={{ color: "var(--red)" }} />
        ) : (
          <Check size={9} strokeWidth={2} style={{ color: "var(--fill-tertiary)" }} />
        )}
      </span>
      <span className="truncate font-mono">{tc.name}</span>
    </div>
  );
}

interface SubAgentCardProps {
  run: SubAgentRunUI;
  onCancel?: (runId: string) => void;
}

export function SubAgentCard({ run, onCancel }: SubAgentCardProps) {
  const [expanded, setExpanded] = useState(false);
  const meta = getTypeMeta(run.subagentType);
  const isActive = run.status === "running" || run.status === "pending";
  const isFailed = run.status === "failed" || run.status === "cancelled";

  return (
    <div
      className="my-2 overflow-hidden rounded-lg"
      style={{
        border: `0.5px solid ${isFailed ? "color-mix(in srgb, var(--red) 25%, transparent)" : `color-mix(in srgb, ${meta.color} 30%, var(--separator))`}`,
        background: isFailed
          ? "color-mix(in srgb, var(--red) 3%, var(--bg-secondary))"
          : "var(--bg-secondary)",
        animation: "slide-up var(--duration-fast) var(--ease-out)",
        maxWidth: "min(100%, 640px)",
      }}
    >
      {/* Header */}
      <button
        onClick={() => setExpanded(!expanded)}
        className="flex w-full items-center gap-2 px-3 py-2 text-left transition-colors duration-100"
        style={{ cursor: "pointer" }}
        aria-expanded={expanded}
      >
        <span className="flex h-4 w-4 shrink-0 items-center justify-center">
          <StatusIndicator status={run.status} />
        </span>

        <span className="flex min-w-0 flex-1 items-center gap-1.5 text-[12px]">
          <span className="shrink-0" style={{ color: meta.color }}>{meta.icon}</span>
          <span className="shrink-0 font-medium" style={{ color: "var(--fill-primary)" }}>
            {meta.label}
          </span>
          <span
            className="min-w-0 truncate text-[11px]"
            style={{ color: "var(--fill-tertiary)" }}
            title={run.task}
          >
            {run.task.length > 60 ? run.task.slice(0, 60) + "…" : run.task}
          </span>
        </span>

        {run.elapsedMs != null && (
          <span className="shrink-0 text-[10px] tabular-nums" style={{ color: "var(--fill-quaternary)" }}>
            {run.elapsedMs < 1000 ? `${run.elapsedMs}ms` : `${(run.elapsedMs / 1000).toFixed(1)}s`}
          </span>
        )}

        {isActive && onCancel && (
          <button
            onClick={(e) => { e.stopPropagation(); onCancel(run.runId); }}
            className="flex h-5 w-5 shrink-0 items-center justify-center rounded transition-colors hover:bg-[var(--fill-quaternary)]"
            title="取消"
            aria-label="取消子智能体"
          >
            <Square size={9} strokeWidth={2} style={{ color: "var(--fill-tertiary)" }} />
          </button>
        )}

        <ChevronRight
          size={10}
          strokeWidth={2}
          className="shrink-0 transition-transform duration-150"
          style={{
            color: "var(--fill-quaternary)",
            transform: expanded ? "rotate(90deg)" : "rotate(0)",
          }}
        />
      </button>

      {/* Collapsed: show tool call chips */}
      {!expanded && run.toolCalls.length > 0 && (
        <div className="flex flex-wrap gap-1 px-3 pb-2">
          {run.toolCalls.slice(0, 6).map((tc: SubAgentToolCall) => (
            <MiniToolCall key={tc.id} tc={tc} />
          ))}
          {run.toolCalls.length > 6 && (
            <span className="self-center text-[10px]" style={{ color: "var(--fill-quaternary)" }}>
              +{run.toolCalls.length - 6} more
            </span>
          )}
        </div>
      )}

      {/* Expanded content */}
      {expanded && (
        <div
          className="px-3 pb-3"
          style={{
            borderTop: "0.5px solid var(--separator)",
            animation: "fade-in var(--duration-instant) var(--ease-out)",
          }}
        >
          {/* Task */}
          <div className="mt-2">
            <span
              className="text-[10px] font-semibold uppercase tracking-wider"
              style={{ color: "var(--fill-quaternary)" }}
            >
              任务
            </span>
            <p className="mt-0.5 text-[11.5px] leading-relaxed" style={{ color: "var(--fill-secondary)" }}>
              {run.task}
            </p>
          </div>

          {/* Streaming content */}
          {run.content && (
            <div className="mt-2">
              <span
                className="text-[10px] font-semibold uppercase tracking-wider"
                style={{ color: "var(--fill-quaternary)" }}
              >
                输出
              </span>
              <pre
                className="mt-1 overflow-x-auto whitespace-pre-wrap break-words rounded-md p-2.5 text-[11px] leading-[1.55]"
                style={{
                  background: "var(--bg-primary)",
                  color: "var(--fill-secondary)",
                  border: "0.5px solid var(--separator)",
                  fontFamily: '"SF Mono","Fira Code",Menlo,Monaco,monospace',
                  maxHeight: "300px",
                  overflowY: "auto",
                }}
              >
                {run.content}
              </pre>
            </div>
          )}

          {/* Tool calls */}
          {run.toolCalls.length > 0 && (
            <div className="mt-2">
              <span
                className="text-[10px] font-semibold uppercase tracking-wider"
                style={{ color: "var(--fill-quaternary)" }}
              >
                工具调用 ({run.toolCalls.length})
              </span>
              <div className="mt-1 space-y-1">
                {run.toolCalls.map((tc: SubAgentToolCall) => (
                  <div
                    key={tc.id}
                    className="flex items-center gap-2 rounded-md px-2 py-1 text-[11px]"
                    style={{
                      background: "var(--bg-primary)",
                      border: "0.5px solid var(--separator)",
                      color: tc.status === "error" ? "var(--red)" : "var(--fill-secondary)",
                    }}
                  >
                    <span className="flex h-3.5 w-3.5 items-center justify-center">
                      {tc.status === "running" ? (
                        <Loader size={10} className="animate-spin" style={{ color: "var(--fill-tertiary)" }} />
                      ) : tc.status === "error" ? (
                        <XIcon size={10} strokeWidth={2} />
                      ) : (
                        <Check size={10} strokeWidth={2} style={{ color: "var(--fill-tertiary)" }} />
                      )}
                    </span>
                    <span className="font-mono font-medium">{tc.name}</span>
                    {tc.args && (
                      <span className="min-w-0 truncate font-mono" style={{ color: "var(--fill-quaternary)" }}>
                        {tc.args.slice(0, 80)}
                      </span>
                    )}
                  </div>
                ))}
              </div>
            </div>
          )}

          {/* Result */}
          {run.result && (
            <div className="mt-2">
              <span
                className="text-[10px] font-semibold uppercase tracking-wider"
                style={{ color: "var(--fill-quaternary)" }}
              >
                结果
              </span>
              <pre
                className="mt-1 overflow-x-auto whitespace-pre-wrap break-words rounded-md p-2.5 text-[11px] leading-[1.55]"
                style={{
                  background: "var(--bg-primary)",
                  color: isFailed ? "var(--red)" : "var(--fill-secondary)",
                  border: "0.5px solid var(--separator)",
                  fontFamily: '"SF Mono","Fira Code",Menlo,Monaco,monospace',
                  maxHeight: "300px",
                  overflowY: "auto",
                }}
              >
                {run.result}
              </pre>
            </div>
          )}

          {/* Stats footer */}
          {(run.toolCallsMade > 0 || run.iterations > 0) && (
            <div className="mt-2 flex gap-3 text-[10px]" style={{ color: "var(--fill-quaternary)" }}>
              {run.toolCallsMade > 0 && <span>{run.toolCallsMade} 次工具调用</span>}
              {run.iterations > 0 && <span>{run.iterations} 轮迭代</span>}
              {run.elapsedMs != null && <span>耗时 {(run.elapsedMs / 1000).toFixed(1)}s</span>}
            </div>
          )}
        </div>
      )}
    </div>
  );
}
