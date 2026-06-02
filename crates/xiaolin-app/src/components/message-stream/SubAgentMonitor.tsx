import { useState, useEffect, useMemo, useCallback } from "react";
import {
  Bot, Search, Terminal, Globe, Wrench, X, ChevronDown, ChevronRight,
  Clock, Zap, Copy, Square,
} from "lucide-react";
import { useActiveSubAgentRuns } from "../../lib/stores";
import { ICON } from "../../lib/ui-tokens";
import type { SubAgentRunUI } from "../../lib/stores/types";
import * as api from "../../lib/api";

const TYPE_META: Record<string, { icon: React.ReactNode; label: string; color: string }> = {
  general: { icon: <Bot {...ICON.sm} />, label: "通用", color: "var(--tint)" },
  explore: { icon: <Search {...ICON.sm} />, label: "探索", color: "#34c759" },
  shell: { icon: <Terminal {...ICON.sm} />, label: "命令", color: "#ff9500" },
  browser: { icon: <Globe {...ICON.sm} />, label: "浏览器", color: "#af52de" },
};

function getTypeMeta(type: string) {
  return TYPE_META[type] ?? { icon: <Wrench {...ICON.sm} />, label: type, color: "var(--fill-tertiary)" };
}

function formatElapsed(ms?: number): string {
  if (!ms) return "0s";
  if (ms < 1000) return `${ms}ms`;
  return `${(ms / 1000).toFixed(1)}s`;
}

function StatusBadge({ status }: { status: SubAgentRunUI["status"] }) {
  const styles: Record<string, { bg: string; color: string; label: string }> = {
    pending: { bg: "var(--bg-tertiary)", color: "var(--fill-tertiary)", label: "等待" },
    running: { bg: "rgba(0, 122, 255, 0.12)", color: "var(--tint)", label: "运行中" },
    completed: { bg: "rgba(52, 199, 89, 0.12)", color: "#34c759", label: "完成" },
    failed: { bg: "rgba(255, 59, 48, 0.12)", color: "#ff3b30", label: "失败" },
    cancelled: { bg: "rgba(142, 142, 147, 0.12)", color: "var(--fill-tertiary)", label: "取消" },
  };
  const s = styles[status] ?? styles.pending;
  return (
    <span
      className="inline-flex items-center rounded-full px-1.5 py-0.5 text-[10px] font-medium"
      style={{ background: s.bg, color: s.color }}
    >
      {status === "running" && (
        <span
          className="mr-1 inline-block h-1.5 w-1.5 rounded-full"
          style={{ background: s.color, animation: "pulse-subtle 1.5s ease-in-out infinite" }}
        />
      )}
      {s.label}
    </span>
  );
}

function RunItem({ run, onCancel }: { run: SubAgentRunUI; onCancel: (id: string) => void }) {
  const [expanded, setExpanded] = useState(false);
  const meta = getTypeMeta(run.subagentType);
  const isActive = run.status === "running" || run.status === "pending";
  const [elapsed, setElapsed] = useState(run.elapsedMs ?? 0);

  useEffect(() => {
    if (!isActive) return;
    const start = Date.now();
    const baseElapsed = run.elapsedMs ?? 0;
    const id = setInterval(() => setElapsed(baseElapsed + (Date.now() - start)), 1000);
    return () => clearInterval(id);
  }, [isActive, run.elapsedMs]);

  const currentTool = useMemo(() => {
    const running = run.toolCalls.find((tc) => tc.status === "running");
    return running?.name;
  }, [run.toolCalls]);

  return (
    <div
      className="rounded-[var(--radius-xs)] border"
      style={{
        borderColor: isActive ? meta.color + "40" : "var(--separator)",
        background: isActive ? meta.color + "08" : "var(--bg-secondary)",
      }}
    >
      <button
        className="flex w-full items-center gap-2 p-2 text-left"
        onClick={() => setExpanded(!expanded)}
      >
        {expanded ? <ChevronDown size={12} /> : <ChevronRight size={12} />}
        <span style={{ color: meta.color }}>{meta.icon}</span>
        <span className="flex-1 truncate text-[11px] font-medium" style={{ color: "var(--fill-primary)" }}>
          {run.task.length > 50 ? run.task.slice(0, 50) + "…" : run.task}
        </span>
        <StatusBadge status={run.status} />
      </button>

      <div className="flex items-center gap-3 px-2 pb-2 text-[10px]" style={{ color: "var(--fill-tertiary)" }}>
        <span className="inline-flex items-center gap-0.5">
          <Clock size={10} /> {formatElapsed(isActive ? elapsed : run.elapsedMs)}
        </span>
        <span className="inline-flex items-center gap-0.5">
          <Zap size={10} /> {run.toolCallsMade} 工具
        </span>
        {currentTool && (
          <span className="truncate" style={{ color: meta.color }}>
            ▸ {currentTool}
          </span>
        )}
        {isActive && (
          <button
            onClick={(e) => { e.stopPropagation(); onCancel(run.runId); }}
            className="ml-auto rounded p-0.5 transition-colors hover:bg-[var(--bg-tertiary)]"
            title="取消"
          >
            <Square size={10} style={{ color: "var(--red)" }} />
          </button>
        )}
      </div>

      {expanded && run.result && (
        <div className="border-t px-2 py-1.5" style={{ borderColor: "var(--separator)" }}>
          <div className="flex items-center justify-between">
            <span className="text-[10px] font-medium" style={{ color: "var(--fill-tertiary)" }}>结果</span>
            <button
              onClick={() => navigator.clipboard.writeText(run.result ?? "")}
              className="rounded p-0.5 transition-colors hover:bg-[var(--bg-tertiary)]"
              title="复制"
            >
              <Copy size={10} style={{ color: "var(--fill-quaternary)" }} />
            </button>
          </div>
          <pre
            className="mt-1 max-h-[120px] overflow-auto whitespace-pre-wrap text-[10px] leading-relaxed"
            style={{ color: "var(--fill-secondary)" }}
          >
            {run.result.length > 500 ? run.result.slice(0, 500) + "…" : run.result}
          </pre>
        </div>
      )}
    </div>
  );
}

export function SubAgentMonitor() {
  const subAgentRuns = useActiveSubAgentRuns();

  const runs = useMemo(() => {
    return Object.values(subAgentRuns).sort((a, b) => {
      const activeA = a.status === "running" || a.status === "pending" ? 0 : 1;
      const activeB = b.status === "running" || b.status === "pending" ? 0 : 1;
      if (activeA !== activeB) return activeA - activeB;
      return (b.elapsedMs ?? 0) - (a.elapsedMs ?? 0);
    });
  }, [subAgentRuns]);

  const activeCount = useMemo(
    () => runs.filter((r) => r.status === "running" || r.status === "pending").length,
    [runs],
  );

  const [visible, setVisible] = useState(false);
  const [manualHide, setManualHide] = useState(false);

  useEffect(() => {
    if (activeCount > 0) {
      setVisible(true);
      setManualHide(false);
    } else if (visible && !manualHide) {
      const timer = setTimeout(() => setVisible(false), 3000);
      return () => clearTimeout(timer);
    }
  }, [activeCount, visible, manualHide]);

  const handleCancel = useCallback(async (runId: string) => {
    try {
      await api.cancelSubAgentRun(runId);
    } catch (e) {
      console.error("Failed to cancel sub-agent:", e);
    }
  }, []);

  if (!visible || runs.length === 0) return null;

  return (
    <div
      className="flex h-full shrink-0 flex-col border-l"
      style={{
        width: 280,
        borderColor: "var(--separator)",
        background: "var(--bg-primary)",
        animation: "slide-in-right var(--duration-normal) var(--ease-out)",
        overflow: "hidden",
      }}
    >
      {/* Header */}
      <div
        className="flex shrink-0 items-center justify-between px-3 py-2"
        style={{ borderBottom: "0.5px solid var(--separator)" }}
      >
        <div className="flex items-center gap-1.5">
          <Bot size={14} style={{ color: "var(--tint)" }} />
          <span className="text-[12px] font-semibold" style={{ color: "var(--fill-primary)" }}>
            子智能体
          </span>
          {activeCount > 0 && (
            <span
              className="rounded-full px-1.5 py-0.5 text-[10px] font-medium"
              style={{ background: "rgba(0, 122, 255, 0.12)", color: "var(--tint)" }}
            >
              {activeCount} 运行中
            </span>
          )}
        </div>
        <button
          onClick={() => { setManualHide(true); setVisible(false); }}
          className="rounded p-0.5 transition-colors hover:bg-[var(--bg-tertiary)]"
          title="隐藏面板"
        >
          <X size={14} style={{ color: "var(--fill-quaternary)" }} />
        </button>
      </div>

      {/* Run list */}
      <div className="flex flex-1 flex-col gap-1.5 overflow-y-auto p-2">
        {runs.map((run) => (
          <RunItem key={run.runId} run={run} onCancel={handleCancel} />
        ))}
      </div>
    </div>
  );
}
