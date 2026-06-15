import { useState, useEffect, useMemo, useCallback, useRef } from "react";
import { useTranslation } from "react-i18next";
import {
  Robot, MagnifyingGlass, Terminal, Globe, Wrench, Check, X as XIcon,
  Clock, Lightning, PaperPlaneRight, CaretDown, CaretRight, Copy, Square,
} from "@phosphor-icons/react";
import { useActiveSubAgentRuns } from "../../lib/stores";
import { useStreamStore } from "../../lib/stores/stream-store";
import { useChatMetaStore } from "../../lib/stores/chat-meta-store";
import type { SubAgentRunUI } from "../../lib/stores/types";
import * as api from "../../lib/api";

function useTypeMeta() {
  const { t } = useTranslation("chat");
  return useMemo(() => {
    const map: Record<string, { icon: React.ReactNode; label: string; color: string }> = {
      general: { icon: <Robot size={12} />, label: t("subAgent_general"), color: "var(--tint)" },
      explore: { icon: <MagnifyingGlass size={12} />, label: t("subAgentCard_explore"), color: "#34c759" },
      shell: { icon: <Terminal size={12} />, label: t("subAgentCard_shell"), color: "#ff9500" },
      browser: { icon: <Globe size={12} />, label: t("subAgent_browser"), color: "#af52de" },
      coordinator: { icon: <Robot size={12} />, label: t("subAgent_coordinator"), color: "var(--tint)" },
    };
    return (type: string) => map[type] ?? { icon: <Wrench size={12} />, label: type, color: "var(--fill-tertiary)" };
  }, [t]);
}

function formatElapsed(ms?: number): string {
  if (!ms) return "—";
  if (ms < 1000) return `${ms}ms`;
  return `${(ms / 1000).toFixed(1)}s`;
}

function RunItem({ run, onCancel }: { run: SubAgentRunUI; onCancel: (id: string) => void }) {
  const { t } = useTranslation("chat");
  const getTypeMeta = useTypeMeta();
  const [expanded, setExpanded] = useState(false);
  const meta = getTypeMeta(run.subagentType);
  const isActive = run.status === "running" || run.status === "pending";
  const isFailed = run.status === "failed" || run.status === "cancelled";
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
        {expanded ? <CaretDown size={10} /> : <CaretRight size={10} />}
        <span style={{ color: meta.color }}>{meta.icon}</span>
        <span className="flex-1 truncate text-[11px] font-medium" style={{ color: "var(--fill-primary)" }}>
          {run.task.length > 50 ? run.task.slice(0, 50) + "…" : run.task}
        </span>
        <span className="shrink-0">
          {isActive ? (
            <span
              className="inline-block h-2 w-2 rounded-full"
              style={{ background: meta.color, animation: "pulse-subtle 1.5s ease-in-out infinite" }}
            />
          ) : isFailed ? (
            <XIcon size={12} style={{ color: "var(--red)" }} />
          ) : (
            <Check size={12} style={{ color: "var(--green)" }} />
          )}
        </span>
      </button>

      <div className="flex items-center gap-3 px-2 pb-2 text-[10px]" style={{ color: "var(--fill-tertiary)" }}>
        <span className="inline-flex items-center gap-0.5">
          <Clock size={9} /> {formatElapsed(isActive ? elapsed : run.elapsedMs)}
        </span>
        <span className="inline-flex items-center gap-0.5">
          <Lightning size={9} /> {(() => {
            const count = Math.max(run.toolCallsMade, run.toolCalls.length);
            return isActive && count === 0 ? t("subAgent_thinking") : t("subAgent_toolsCount", { count });
          })()}
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

export function SubAgentsTabContent() {
  const { t } = useTranslation("chat");
  const subAgentRuns = useActiveSubAgentRuns();
  const [steerInput, setSteerInput] = useState("");
  const [steerSending, setSteerSending] = useState(false);
  const pollRef = useRef<ReturnType<typeof setInterval> | null>(null);

  // Poll backend for sub-agent status when there are "running" runs but
  // the turn event stream may have already ended.
  const activeChatId = useChatMetaStore((s) => s.activeChatId);
  const subAgentComplete = useStreamStore((s) => s.subAgentComplete);

  useEffect(() => {
    const activeRuns = Object.values(subAgentRuns).filter(
      (r) => r.status === "running" || r.status === "pending"
    );
    if (activeRuns.length === 0) {
      if (pollRef.current) { clearInterval(pollRef.current); pollRef.current = null; }
      return;
    }

    const poll = async () => {
      try {
        const runs = await api.listSubAgentRuns();
        for (const run of runs) {
          const local = subAgentRuns[run.runId];
          if (local && (local.status === "running" || local.status === "pending") && run.status !== local.status) {
            subAgentComplete(
              activeChatId,
              run.runId,
              run.status,
              run.result ?? undefined,
              run.toolCallsMade,
              run.iterations,
              run.elapsedMs,
            );
          }
        }
      } catch { /* ignore poll errors */ }
    };

    pollRef.current = setInterval(poll, 3000);
    return () => { if (pollRef.current) { clearInterval(pollRef.current); pollRef.current = null; } };
  }, [subAgentRuns, activeChatId, subAgentComplete]);

  const { coordinatorRun, allRuns } = useMemo(() => {
    const runs = Object.values(subAgentRuns);
    const coord = runs.find((r) => r.subagentType === "coordinator");
    const sorted = [...runs].sort((a, b) => {
      const activeA = (a.status === "running" || a.status === "pending") ? 0 : 1;
      const activeB = (b.status === "running" || b.status === "pending") ? 0 : 1;
      if (activeA !== activeB) return activeA - activeB;
      return (b.elapsedMs ?? 0) - (a.elapsedMs ?? 0);
    });
    return { coordinatorRun: coord, allRuns: sorted };
  }, [subAgentRuns]);

  const handleCancel = useCallback(async (runId: string) => {
    try {
      await api.cancelSubAgentRun(runId);
    } catch (e) {
      console.error("Failed to cancel sub-agent:", e);
    }
  }, []);

  const handleSteer = useCallback(async () => {
    if (!coordinatorRun || !steerInput.trim() || steerSending) return;
    setSteerSending(true);
    try {
      await api.sendSteeringMessage(coordinatorRun.runId, steerInput.trim(), "high");
      setSteerInput("");
    } catch (e) {
      console.error("Failed to steer coordinator:", e);
    } finally {
      setSteerSending(false);
    }
  }, [coordinatorRun, steerInput, steerSending]);

  const activeCount = allRuns.filter((r) => r.status === "running" || r.status === "pending").length;
  const coordIsActive = coordinatorRun && (coordinatorRun.status === "running" || coordinatorRun.status === "pending");

  if (allRuns.length === 0) {
    return (
      <div className="flex flex-col items-center justify-center h-full px-4 text-center">
        <Robot size={32} style={{ color: "var(--fill-quaternary)", opacity: 0.5 }} />
        <p className="mt-2 text-[12px]" style={{ color: "var(--fill-tertiary)" }}>
          {t("coordinator_empty")}
        </p>
      </div>
    );
  }

  return (
    <div className="flex flex-col h-full">
      {/* Coordinator header (if exists) */}
      {coordinatorRun && (
        <div className="shrink-0 px-3 py-2 border-b" style={{ borderColor: "var(--separator)" }}>
          <div className="flex items-center gap-2">
            <Robot size={14} style={{ color: "var(--tint)" }} />
            <span className="text-[12px] font-semibold" style={{ color: "var(--fill-primary)" }}>
              {t("coordinator_title")}
            </span>
            {coordIsActive && (
              <span
                className="inline-block h-2 w-2 rounded-full"
                style={{ background: "var(--tint)", animation: "pulse-subtle 1.5s ease-in-out infinite" }}
              />
            )}
          </div>
          <p className="mt-0.5 text-[11px] truncate" style={{ color: "var(--fill-tertiary)" }}>
            {coordinatorRun.task}
          </p>
          <div className="flex items-center gap-3 mt-1 text-[10px]" style={{ color: "var(--fill-quaternary)" }}>
            <span>{t("subAgent_runningSummary", { count: activeCount })}</span>
            <span>{formatElapsed(coordinatorRun.elapsedMs)}</span>
          </div>
        </div>
      )}

      {/* Run list */}
      <div className="flex-1 overflow-y-auto px-3 py-2 space-y-1.5">
        {allRuns.map((run) => (
          <RunItem key={run.runId} run={run} onCancel={handleCancel} />
        ))}
      </div>

      {/* Steering input (only when coordinator is active) */}
      {coordIsActive && (
        <div className="shrink-0 border-t px-3 py-2" style={{ borderColor: "var(--separator)" }}>
          <div
            className="flex items-center gap-1.5 rounded-md border px-2 py-1.5"
            style={{ borderColor: "var(--separator)", background: "var(--bg-primary)" }}
          >
            <input
              type="text"
              className="flex-1 bg-transparent text-[11px] outline-none"
              style={{ color: "var(--fill-primary)" }}
              placeholder={t("coordinator_steerPlaceholder")}
              value={steerInput}
              onChange={(e) => setSteerInput(e.target.value)}
              onKeyDown={(e) => { if (e.key === "Enter" && !e.shiftKey) { e.preventDefault(); handleSteer(); } }}
              disabled={steerSending}
            />
            <button
              onClick={handleSteer}
              disabled={!steerInput.trim() || steerSending}
              className="shrink-0 rounded p-1 transition-colors hover:bg-[var(--bg-tertiary)] disabled:opacity-30"
            >
              <PaperPlaneRight size={14} style={{ color: "var(--tint)" }} />
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
