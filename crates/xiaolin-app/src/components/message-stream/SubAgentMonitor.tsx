import { useState, useEffect, useMemo, useCallback, useRef } from "react";
import { useTranslation } from "react-i18next";
import {
  Robot, MagnifyingGlass, Terminal, Globe, Wrench, X, CaretDown, CaretRight, CaretUp,
  Clock, Lightning, Copy, Square,
} from "@phosphor-icons/react";
import { useActiveSubAgentRuns } from "../../lib/stores";
import type { SubAgentRunUI } from "../../lib/stores/types";
import * as api from "../../lib/api";

function useTypeMeta() {
  const { t } = useTranslation("chat");
  return useMemo(() => {
    const map: Record<string, { icon: React.ReactNode; label: string; color: string }> = {
      general: { icon: <Robot />, label: t("subAgent_general"), color: "var(--tint)" },
      explore: { icon: <MagnifyingGlass />, label: t("subAgent_explore"), color: "#34c759" },
      shell: { icon: <Terminal />, label: t("subAgent_shell"), color: "#ff9500" },
      browser: { icon: <Globe />, label: t("subAgent_browser"), color: "#af52de" },
    };
    return (type: string) => map[type] ?? { icon: <Wrench />, label: type, color: "var(--fill-tertiary)" };
  }, [t]);
}

function formatElapsed(ms?: number): string {
  if (!ms) return "0s";
  if (ms < 1000) return `${ms}ms`;
  return `${(ms / 1000).toFixed(1)}s`;
}

function StatusBadge({ status }: { status: SubAgentRunUI["status"] }) {
  const { t } = useTranslation("chat");
  const styles: Record<string, { bg: string; color: string; label: string }> = {
    pending: { bg: "var(--bg-tertiary)", color: "var(--fill-tertiary)", label: t("subAgent_status_pending") },
    running: { bg: "rgba(0, 122, 255, 0.12)", color: "var(--tint)", label: t("subAgent_status_running") },
    completed: { bg: "rgba(52, 199, 89, 0.12)", color: "#34c759", label: t("subAgent_status_completed") },
    failed: { bg: "rgba(255, 59, 48, 0.12)", color: "#ff3b30", label: t("subAgent_status_failed") },
    cancelled: { bg: "rgba(142, 142, 147, 0.12)", color: "var(--fill-tertiary)", label: t("subAgent_status_cancelled") },
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
  const { t } = useTranslation("chat");
  const getTypeMeta = useTypeMeta();
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
        {expanded ? <CaretDown size={12} /> : <CaretRight size={12} />}
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
          <Lightning size={10} /> {t("subAgent_toolsCount", { count: run.toolCallsMade })}
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
            title={t("cancel", { ns: "common" })}
          >
            <Square size={10} style={{ color: "var(--red)" }} />
          </button>
        )}
      </div>

      {expanded && (
        <div className="border-t px-2 py-1.5 space-y-1.5" style={{ borderColor: "var(--separator)" }}>
          {run.notifications.length > 0 && (
            <div>
              <span className="text-[10px] font-medium" style={{ color: "var(--fill-tertiary)" }}>
                {t("subAgent_notifications")}
              </span>
              <div className="mt-0.5 max-h-[80px] overflow-y-auto space-y-0.5">
                {run.notifications.slice(-5).map((n, i) => (
                  <div
                    key={i}
                    className="flex items-start gap-1 text-[10px] leading-tight"
                    style={{ color: "var(--fill-secondary)" }}
                  >
                    <span className="shrink-0" style={{ color: "var(--fill-quaternary)" }}>
                      {new Date(n.timestamp).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" })}
                    </span>
                    <span className="min-w-0 break-words">{n.message}</span>
                  </div>
                ))}
              </div>
            </div>
          )}
          {run.result && (
            <div>
              <div className="flex items-center justify-between">
                <span className="text-[10px] font-medium" style={{ color: "var(--fill-tertiary)" }}>{t("subAgent_result")}</span>
                <button
                  onClick={() => navigator.clipboard.writeText(run.result ?? "")}
                  className="rounded p-0.5 transition-colors hover:bg-[var(--bg-tertiary)]"
                  title={t("copy", { ns: "common" })}
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
      )}
    </div>
  );
}

const MIN_DRAWER_H = 120;
const MAX_DRAWER_H = 400;
const DEFAULT_DRAWER_H = 240;
const SUMMARY_H = 36;

export function SubAgentMonitor() {
  const { t } = useTranslation("chat");
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
  const [collapsed, setCollapsed] = useState(false);
  const [drawerHeight, setDrawerHeight] = useState(DEFAULT_DRAWER_H);
  const [dragging, setDragging] = useState(false);
  const prevActiveCount = useRef(activeCount);

  useEffect(() => {
    if (activeCount > 0 && !visible) {
      setVisible(true);
      setCollapsed(false);
    } else if (activeCount === 0 && prevActiveCount.current > 0) {
      const timer = setTimeout(() => setCollapsed(true), 3000);
      return () => clearTimeout(timer);
    }
    prevActiveCount.current = activeCount;
  }, [activeCount, visible]);

  const handleCancel = useCallback(async (runId: string) => {
    try {
      await api.cancelSubAgentRun(runId);
    } catch (e) {
      console.error("Failed to cancel sub-agent:", e);
    }
  }, []);

  const handleResizePointerDown = useCallback((e: React.PointerEvent) => {
    e.preventDefault();
    (e.target as HTMLElement).setPointerCapture(e.pointerId);
    setDragging(true);
  }, []);

  useEffect(() => {
    if (!dragging) return;
    const startY = { current: 0 };
    const startH = { current: drawerHeight };
    const handleMove = (e: PointerEvent) => {
      if (startY.current === 0) {
        startY.current = e.clientY;
        startH.current = drawerHeight;
        return;
      }
      const delta = startY.current - e.clientY;
      setDrawerHeight(Math.min(MAX_DRAWER_H, Math.max(MIN_DRAWER_H, startH.current + delta)));
    };
    const handleUp = () => setDragging(false);
    window.addEventListener("pointermove", handleMove);
    window.addEventListener("pointerup", handleUp);
    document.body.style.cursor = "row-resize";
    document.body.style.userSelect = "none";
    return () => {
      window.removeEventListener("pointermove", handleMove);
      window.removeEventListener("pointerup", handleUp);
      document.body.style.cursor = "";
      document.body.style.userSelect = "";
    };
  }, [dragging, drawerHeight]);

  if (!visible || runs.length === 0) return null;

  const completedCount = runs.filter((r) => r.status === "completed").length;
  const summaryText = activeCount > 0
    ? t("subAgent_runningSummary", { count: activeCount })
    : t("subAgent_completedSummary", { count: completedCount });

  return (
    <div
      className="absolute inset-x-0 bottom-0 z-10 flex flex-col border-t"
      style={{
        height: collapsed ? SUMMARY_H : drawerHeight,
        borderColor: "var(--separator)",
        background: "var(--bg-primary)",
        animation: "slide-up var(--duration-normal) var(--ease-out)",
        transition: collapsed ? "height var(--duration-normal) var(--ease-in-out)" : undefined,
      }}
    >
      {!collapsed && (
        <div
          className="absolute -top-1 inset-x-0 z-10"
          style={{ height: 8, cursor: "row-resize" }}
          onPointerDown={handleResizePointerDown}
        >
          <div className="mx-auto mt-1 h-1 w-8 rounded-full" style={{ background: "var(--fill-quaternary)", opacity: 0.4 }} />
        </div>
      )}

      <div
        className="flex shrink-0 items-center justify-between px-3"
        style={{ height: SUMMARY_H, cursor: "pointer" }}
        onClick={() => setCollapsed(!collapsed)}
      >
        <div className="flex items-center gap-1.5">
          <Robot size={14} style={{ color: "var(--tint)" }} />
          <span className="text-[12px] font-semibold" style={{ color: "var(--fill-primary)" }}>
            {t("subAgent_title")}
          </span>
          <span
            className="rounded-full px-1.5 py-0.5 text-[10px] font-medium"
            style={{
              background: activeCount > 0 ? "rgba(0, 122, 255, 0.12)" : "rgba(52, 199, 89, 0.12)",
              color: activeCount > 0 ? "var(--tint)" : "#34c759",
            }}
          >
            {summaryText}
          </span>
        </div>
        <div className="flex items-center gap-1">
          <button className="rounded p-0.5" style={{ color: "var(--fill-quaternary)" }}>
            {collapsed ? <CaretUp size={14} /> : <CaretDown size={14} />}
          </button>
          <button
            onClick={(e) => { e.stopPropagation(); setVisible(false); }}
            className="rounded p-0.5 transition-colors hover:bg-[var(--bg-tertiary)]"
            title={t("subAgent_hidePanel")}
          >
            <X size={14} style={{ color: "var(--fill-quaternary)" }} />
          </button>
        </div>
      </div>

      {!collapsed && (
        <div className="flex flex-1 flex-col gap-1.5 overflow-y-auto px-3 pb-2">
          {runs.map((run) => (
            <RunItem key={run.runId} run={run} onCancel={handleCancel} />
          ))}
        </div>
      )}
    </div>
  );
}
