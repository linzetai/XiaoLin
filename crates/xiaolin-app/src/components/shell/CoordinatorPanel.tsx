import { useMemo, useCallback, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  Robot, MagnifyingGlass, Terminal, Globe, Wrench, Check, X as XIcon,
  Clock, Lightning, PaperPlaneRight,
} from "@phosphor-icons/react";
import { useActiveSubAgentRuns } from "../../lib/stores";
import type { SubAgentRunUI } from "../../lib/stores/types";
import * as api from "../../lib/api";

function useTypeMeta() {
  const { t } = useTranslation("chat");
  return useMemo(() => {
    const map: Record<string, { icon: React.ReactNode; label: string; color: string }> = {
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

function WorkerRow({ run }: { run: SubAgentRunUI }) {
  const { t } = useTranslation("chat");
  const getTypeMeta = useTypeMeta();
  const meta = getTypeMeta(run.subagentType);
  const isActive = run.status === "running" || run.status === "pending";
  const isFailed = run.status === "failed" || run.status === "cancelled";

  return (
    <div
      className="flex items-center gap-2 rounded px-2 py-1.5"
      style={{
        background: isActive ? meta.color + "08" : "var(--bg-secondary)",
        border: `0.5px solid ${isActive ? meta.color + "30" : "var(--separator)"}`,
      }}
    >
      <span style={{ color: meta.color }}>{meta.icon}</span>
      <div className="flex-1 min-w-0">
        <div className="flex items-center gap-1.5">
          <span className="text-[11px] font-medium truncate" style={{ color: "var(--fill-primary)" }}>
            {run.task.length > 40 ? run.task.slice(0, 40) + "…" : run.task}
          </span>
        </div>
        <div className="flex items-center gap-2 mt-0.5 text-[10px]" style={{ color: "var(--fill-quaternary)" }}>
          <span className="inline-flex items-center gap-0.5">
            <Clock size={9} /> {formatElapsed(run.elapsedMs)}
          </span>
          <span className="inline-flex items-center gap-0.5">
            <Lightning size={9} /> {(() => {
              const count = Math.max(run.toolCallsMade, run.toolCalls.length);
              return isActive && count === 0 ? t("subAgent_thinking") : count;
            })()}
          </span>
        </div>
      </div>
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
    </div>
  );
}

export function CoordinatorTabContent() {
  const { t } = useTranslation("chat");
  const subAgentRuns = useActiveSubAgentRuns();
  const [steerInput, setSteerInput] = useState("");
  const [steerSending, setSteerSending] = useState(false);

  const { coordinatorRun, workerRuns } = useMemo(() => {
    const runs = Object.values(subAgentRuns);
    const coord = runs.find((r) => r.subagentType === "coordinator");
    const workers = runs.filter((r) => r.subagentType !== "coordinator")
      .sort((a, b) => {
        const activeA = (a.status === "running" || a.status === "pending") ? 0 : 1;
        const activeB = (b.status === "running" || b.status === "pending") ? 0 : 1;
        if (activeA !== activeB) return activeA - activeB;
        return (b.elapsedMs ?? 0) - (a.elapsedMs ?? 0);
      });
    return { coordinatorRun: coord, workerRuns: workers };
  }, [subAgentRuns]);

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

  if (!coordinatorRun) {
    return (
      <div className="flex flex-col items-center justify-center h-full px-4 text-center">
        <Robot size={32} style={{ color: "var(--fill-quaternary)", opacity: 0.5 }} />
        <p className="mt-2 text-[12px]" style={{ color: "var(--fill-tertiary)" }}>
          {t("coordinator_empty")}
        </p>
      </div>
    );
  }

  const coordIsActive = coordinatorRun.status === "running" || coordinatorRun.status === "pending";
  const activeWorkers = workerRuns.filter((r) => r.status === "running" || r.status === "pending").length;
  const completedWorkers = workerRuns.filter((r) => r.status === "completed").length;

  return (
    <div className="flex flex-col h-full">
      {/* Header */}
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
          <span>{t("coordinator_workers", { active: activeWorkers, completed: completedWorkers, total: workerRuns.length })}</span>
          <span>{formatElapsed(coordinatorRun.elapsedMs)}</span>
        </div>
      </div>

      {/* Worker list */}
      <div className="flex-1 overflow-y-auto px-3 py-2 space-y-1.5">
        {workerRuns.length === 0 ? (
          <p className="text-[11px] text-center py-4" style={{ color: "var(--fill-quaternary)" }}>
            {t("coordinator_noWorkers")}
          </p>
        ) : (
          workerRuns.map((run) => <WorkerRow key={run.runId} run={run} />)
        )}
      </div>

      {/* Notifications */}
      {coordinatorRun.notifications.length > 0 && (
        <div className="shrink-0 border-t px-3 py-1.5 max-h-[100px] overflow-y-auto" style={{ borderColor: "var(--separator)" }}>
          {coordinatorRun.notifications.slice(-4).map((n, i) => (
            <div key={i} className="text-[10px] leading-tight py-0.5" style={{ color: "var(--fill-secondary)" }}>
              <span style={{ color: "var(--fill-quaternary)" }}>
                {new Date(n.timestamp).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}
              </span>{" "}
              {n.message.length > 80 ? n.message.slice(0, 80) + "…" : n.message}
            </div>
          ))}
        </div>
      )}

      {/* Steering input */}
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
