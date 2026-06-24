import { useState, useEffect, useMemo, useCallback, useRef, lazy, Suspense } from "react";
import { useTranslation } from "react-i18next";
import {
  Robot, MagnifyingGlass, Terminal, Globe, Wrench, Check, X as XIcon,
  Clock, Lightning, PaperPlaneRight, CaretDown, CaretRight, Copy, Square,
  FileText, FastForward, SkipForward, Question, CircleNotch,
} from "@phosphor-icons/react";
import { useActiveSubAgentRuns } from "../../lib/stores";
import { useStreamStore } from "../../lib/stores/stream-store";
import { useChatMetaStore } from "../../lib/stores/chat-meta-store";
import type { SubAgentRunUI } from "../../lib/stores/types";
import { useElapsedTimer, formatElapsed } from "../../lib/hooks/useElapsedTimer";
import * as api from "../../lib/api";

const MarkdownContent = lazy(() =>
  import("../message-stream/MarkdownContent").then((m) => ({ default: m.MarkdownContent })),
);

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

function RunItem({ run, onCancel }: { run: SubAgentRunUI; onCancel: (id: string) => void }) {
  const { t } = useTranslation("chat");
  const getTypeMeta = useTypeMeta();
  const [expanded, setExpanded] = useState(false);
  const meta = getTypeMeta(run.subagentType);
  const isActive = run.status === "running" || run.status === "pending";
  const isFailed = run.status === "failed" || run.status === "cancelled";
  const elapsed = useElapsedTimer(isActive, run.elapsedMs ?? 0);

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
          <Clock size={9} /> {formatElapsed(elapsed)}
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
              {isFailed ? (
                <pre
                  className="mt-1 max-h-[120px] overflow-auto whitespace-pre-wrap text-[10px] leading-relaxed"
                  style={{ color: "var(--red)" }}
                >
                  {run.result.length > 500 ? run.result.slice(0, 500) + "…" : run.result}
                </pre>
              ) : (
                <div className="subagent-md mt-1 max-h-[160px] overflow-auto">
                  <Suspense fallback={<div className="animate-pulse rounded py-1" style={{ background: "var(--bg-tertiary)", height: 14 }} />}>
                    <MarkdownContent content={run.result} />
                  </Suspense>
                </div>
              )}
            </div>
          )}
        </div>
      )}
    </div>
  );
}

/** Sort rank for workers: running first, then failed, then completed. */
function statusRank(status: SubAgentRunUI["status"]): number {
  if (status === "running" || status === "pending") return 0;
  if (status === "failed" || status === "cancelled") return 1;
  return 2;
}

function sortWorkers(workers: SubAgentRunUI[]): SubAgentRunUI[] {
  return [...workers].sort((a, b) => {
    const r = statusRank(a.status) - statusRank(b.status);
    if (r !== 0) return r;
    return (b.elapsedMs ?? 0) - (a.elapsedMs ?? 0);
  });
}

/** A worker run rendered as a tree child with a CSS connector line. */
function WorkerRow({ run, isLast, onCancel }: { run: SubAgentRunUI; isLast: boolean; onCancel: (id: string) => void }) {
  return (
    <div className="relative pl-4">
      {/* vertical trunk (stops at the elbow for the last child) */}
      <span
        aria-hidden
        className="absolute left-0 top-0 w-px"
        style={{ background: "var(--separator)", height: isLast ? "13px" : "100%" }}
      />
      {/* horizontal elbow */}
      <span
        aria-hidden
        className="absolute left-0 top-[13px] h-px w-3"
        style={{ background: "var(--separator)" }}
      />
      <RunItem run={run} onCancel={onCancel} />
    </div>
  );
}

interface SteeringEntry {
  id: string;
  runId: string;
  label: string;
  message: string;
  priority: "normal" | "high";
  status: "sending" | "sent" | "failed";
  timestamp: number;
}

/** Enhanced steering controls: target + priority + quick actions + history. */
function SteeringBar({ candidates, defaultTargetId }: { candidates: SubAgentRunUI[]; defaultTargetId: string }) {
  const { t } = useTranslation("chat");
  const [steerInput, setSteerInput] = useState("");
  const [status, setStatus] = useState<"idle" | "sending" | "sent" | "failed">("idle");
  const [priority, setPriority] = useState<"normal" | "high">("high");
  const [targetId, setTargetId] = useState<string | null>(null);
  const [history, setHistory] = useState<SteeringEntry[]>([]);
  const [showHistory, setShowHistory] = useState(false);
  const statusTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => () => { if (statusTimerRef.current) clearTimeout(statusTimerRef.current); }, []);

  const effectiveTarget =
    targetId && candidates.some((c) => c.runId === targetId)
      ? targetId
      : defaultTargetId;

  const targetLabel = useMemo(() => {
    const run = candidates.find((c) => c.runId === effectiveTarget);
    if (!run) return effectiveTarget;
    if (run.subagentType === "coordinator") return t("steer_targetCoordinator");
    return run.task.length > 24 ? run.task.slice(0, 24) + "…" : run.task;
  }, [candidates, effectiveTarget, t]);

  const handleSend = useCallback(async () => {
    const msg = steerInput.trim();
    if (!msg || status === "sending" || !effectiveTarget) return;
    const entryId = `${Date.now()}-${Math.random().toString(36).slice(2, 7)}`;
    setStatus("sending");
    const entry: SteeringEntry = {
      id: entryId, runId: effectiveTarget, label: targetLabel, message: msg, priority, status: "sending", timestamp: Date.now(),
    };
    setHistory((h) => [entry, ...h].slice(0, 20));
    try {
      await api.sendSteeringMessage(effectiveTarget, msg, priority);
      setSteerInput("");
      setStatus("sent");
      setHistory((h) => h.map((e) => (e.id === entryId ? { ...e, status: "sent" } : e)));
      statusTimerRef.current = setTimeout(() => setStatus("idle"), 1500);
    } catch (e) {
      console.error("Failed to send steering message:", e);
      setStatus("failed");
      setHistory((h) => h.map((e) => (e.id === entryId ? { ...e, status: "failed" } : e)));
      statusTimerRef.current = setTimeout(() => setStatus("idle"), 2500);
    }
  }, [steerInput, status, effectiveTarget, targetLabel, priority]);

  const applyQuick = useCallback((tmpl: string) => {
    setSteerInput((prev) => (prev ? prev : tmpl));
  }, []);

  const quickActions: { id: string; icon: React.ReactNode; label: string; tmpl: string }[] = [
    { id: "focus", icon: <FileText size={11} />, label: t("steer_quickFocus"), tmpl: t("steer_tmplFocus") },
    { id: "speedup", icon: <FastForward size={11} />, label: t("steer_quickSpeedup"), tmpl: t("steer_tmplSpeedup") },
    { id: "skip", icon: <SkipForward size={11} />, label: t("steer_quickSkip"), tmpl: t("steer_tmplSkip") },
    { id: "explain", icon: <Question size={11} />, label: t("steer_quickExplain"), tmpl: t("steer_tmplExplain") },
  ];

  return (
    <div className="shrink-0 border-t px-3 py-2 space-y-1.5" style={{ borderColor: "var(--separator)" }}>
      {/* Target + priority */}
      <div className="flex items-center gap-2 text-[10px]" style={{ color: "var(--fill-tertiary)" }}>
        {candidates.length > 1 && (
          <label className="flex items-center gap-1">
            <span>{t("steer_target")}</span>
            <select
              value={effectiveTarget}
              onChange={(e) => setTargetId(e.target.value)}
              className="rounded border bg-transparent px-1 py-0.5 text-[10px] outline-none"
              style={{ borderColor: "var(--separator)", color: "var(--fill-primary)" }}
            >
              {candidates.map((c) => (
                <option key={c.runId} value={c.runId} style={{ background: "var(--bg-primary)" }}>
                  {c.subagentType === "coordinator"
                    ? t("steer_targetCoordinator")
                    : c.task.length > 30 ? c.task.slice(0, 30) + "…" : c.task}
                </option>
              ))}
            </select>
          </label>
        )}
        <span className="ml-auto flex items-center gap-1">
          <span>{t("steer_priority")}</span>
          {(["normal", "high"] as const).map((p) => (
            <button
              key={p}
              onClick={() => setPriority(p)}
              aria-pressed={priority === p}
              className="rounded px-1.5 py-0.5 transition-colors"
              style={{
                background: priority === p ? "color-mix(in srgb, var(--tint) 14%, transparent)" : "transparent",
                color: priority === p ? "var(--tint)" : "var(--fill-quaternary)",
                fontWeight: priority === p ? 600 : 400,
              }}
            >
              {p === "high" ? t("steer_priorityHigh") : t("steer_priorityNormal")}
            </button>
          ))}
        </span>
      </div>

      {/* Quick actions */}
      <div className="flex flex-wrap gap-1">
        {quickActions.map((qa) => (
          <button
            key={qa.id}
            onClick={() => applyQuick(qa.tmpl)}
            className="inline-flex items-center gap-1 rounded border px-1.5 py-0.5 text-[10px] transition-colors hover:bg-[var(--bg-tertiary)]"
            style={{ borderColor: "var(--separator)", color: "var(--fill-tertiary)" }}
            title={qa.label}
          >
            {qa.icon}
            {qa.label}
          </button>
        ))}
      </div>

      {/* Input + send */}
      <div
        className="flex items-center gap-1.5 rounded-md border px-2 py-1.5"
        style={{
          borderColor: status === "failed" ? "var(--red)" : "var(--separator)",
          background: "var(--bg-primary)",
        }}
      >
        <input
          type="text"
          className="flex-1 bg-transparent text-[11px] outline-none"
          style={{ color: "var(--fill-primary)" }}
          placeholder={t("coordinator_steerPlaceholder")}
          value={steerInput}
          onChange={(e) => setSteerInput(e.target.value)}
          onKeyDown={(e) => { if (e.key === "Enter" && !e.shiftKey) { e.preventDefault(); handleSend(); } }}
          disabled={status === "sending"}
        />
        <span className="shrink-0">
          {status === "sending" ? (
            <CircleNotch size={14} style={{ color: "var(--tint)", animation: "spin 0.8s linear infinite" }} />
          ) : status === "sent" ? (
            <Check size={14} style={{ color: "var(--green)" }} />
          ) : (
            <button
              onClick={handleSend}
              disabled={!steerInput.trim()}
              className="rounded p-1 transition-colors hover:bg-[var(--bg-tertiary)] disabled:opacity-30"
              aria-label={t("subAgentCard_steerSend")}
            >
              <PaperPlaneRight size={14} style={{ color: status === "failed" ? "var(--red)" : "var(--tint)" }} />
            </button>
          )}
        </span>
      </div>

      {/* History */}
      {history.length > 0 && (
        <div>
          <button
            onClick={() => setShowHistory((s) => !s)}
            className="flex items-center gap-1 text-[10px]"
            style={{ color: "var(--fill-quaternary)" }}
          >
            {showHistory ? <CaretDown size={9} /> : <CaretRight size={9} />}
            {t("steer_history")} ({history.length})
          </button>
          {showHistory && (
            <div className="mt-0.5 max-h-[96px] overflow-y-auto space-y-0.5">
              {history.map((e) => (
                <div key={e.id} className="flex items-start gap-1 text-[10px] leading-tight" style={{ color: "var(--fill-secondary)" }}>
                  <span className="shrink-0" style={{ color: "var(--fill-quaternary)" }}>
                    {new Date(e.timestamp).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}
                  </span>
                  <span
                    className="shrink-0"
                    style={{ color: e.status === "sent" ? "var(--green)" : e.status === "failed" ? "var(--red)" : "var(--fill-quaternary)" }}
                  >
                    {e.status === "sent" ? t("steer_statusSent") : e.status === "failed" ? t("steer_statusFailed") : t("steer_statusSending")}
                  </span>
                  {candidates.length > 1 && (
                    <span className="shrink-0 truncate max-w-[72px]" style={{ color: "var(--tint)" }} title={e.label}>
                      {e.label}
                    </span>
                  )}
                  <span className="min-w-0 break-words">{e.message}</span>
                </div>
              ))}
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

  const { coordinatorRun, workers, allRuns, stats } = useMemo(() => {
    const runs = Object.values(subAgentRuns);
    const coord = runs.find((r) => r.subagentType === "coordinator");
    const workerRuns = sortWorkers(runs.filter((r) => r.subagentType !== "coordinator"));
    const sortedAll = sortWorkers(runs);
    const s = {
      total: workerRuns.length,
      running: workerRuns.filter((r) => r.status === "running" || r.status === "pending").length,
      failed: workerRuns.filter((r) => r.status === "failed" || r.status === "cancelled").length,
      done: workerRuns.filter((r) => r.status === "completed").length,
    };
    return { coordinatorRun: coord, workers: workerRuns, allRuns: sortedAll, stats: s };
  }, [subAgentRuns]);

  const handleCancel = useCallback(async (runId: string) => {
    try {
      await api.cancelSubAgentRun(runId);
    } catch (e) {
      console.error("Failed to cancel sub-agent:", e);
    }
  }, []);

  const activeRuns = useMemo(
    () => allRuns.filter((r) => r.status === "running" || r.status === "pending"),
    [allRuns],
  );
  const coordIsActive = !!coordinatorRun && (coordinatorRun.status === "running" || coordinatorRun.status === "pending");
  const steerDefaultTarget = coordIsActive ? coordinatorRun!.runId : activeRuns[0]?.runId;

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
      {/* Coordinator header with aggregate stats (if coordinator exists) */}
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
            <span className="ml-auto text-[10px] tabular-nums" style={{ color: "var(--fill-quaternary)" }}>
              {formatElapsed(coordinatorRun.elapsedMs)}
            </span>
          </div>
          <p className="mt-0.5 text-[11px] truncate" style={{ color: "var(--fill-tertiary)" }}>
            {coordinatorRun.task}
          </p>
          <div className="flex flex-wrap items-center gap-x-3 gap-y-0.5 mt-1 text-[10px]">
            <span style={{ color: "var(--fill-tertiary)" }}>{t("coordinator_statWorkers", { count: stats.total })}</span>
            {stats.running > 0 && <span style={{ color: "var(--tint)" }}>{t("coordinator_statRunning", { count: stats.running })}</span>}
            {stats.failed > 0 && <span style={{ color: "var(--red)" }}>{t("coordinator_statFailed", { count: stats.failed })}</span>}
            {stats.done > 0 && <span style={{ color: "var(--green)" }}>{t("coordinator_statDone", { count: stats.done })}</span>}
          </div>
        </div>
      )}

      {/* Run list — tree (with coordinator) or flat list */}
      <div className="flex-1 overflow-y-auto px-3 py-2">
        {coordinatorRun ? (
          workers.length > 0 ? (
            <div>
              <div className="mb-1 text-[10px] font-semibold uppercase tracking-wider" style={{ color: "var(--fill-quaternary)" }}>
                {t("coordinator_workersGroup")}
              </div>
              <div className="space-y-1.5">
                {workers.map((run, i) => (
                  <WorkerRow key={run.runId} run={run} isLast={i === workers.length - 1} onCancel={handleCancel} />
                ))}
              </div>
            </div>
          ) : (
            <p className="text-[11px]" style={{ color: "var(--fill-quaternary)" }}>{t("coordinator_noWorkers")}</p>
          )
        ) : (
          <div className="space-y-1.5">
            {allRuns.map((run) => (
              <RunItem key={run.runId} run={run} onCancel={handleCancel} />
            ))}
          </div>
        )}
      </div>

      {/* Enhanced steering — shown whenever there is at least one active run */}
      {activeRuns.length > 0 && steerDefaultTarget && (
        <SteeringBar candidates={activeRuns} defaultTargetId={steerDefaultTarget} />
      )}
    </div>
  );
}
