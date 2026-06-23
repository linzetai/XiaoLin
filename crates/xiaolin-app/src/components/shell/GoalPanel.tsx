import { useState, useCallback, useMemo, useRef, useEffect } from "react";
import { useTranslation } from "react-i18next";
import {
  Crosshair,
  Pause,
  Play,
  X,
  Plus,
  PencilSimple,
  Check,
  WarningCircle,
} from "@phosphor-icons/react";
import { useGoalStore } from "../../lib/stores/goal-store";
import { useChatMetaStore } from "../../lib/stores/chat-meta-store";
import type { GoalData } from "../../lib/stores/types";
import * as transport from "../../lib/transport";

function formatElapsed(seconds: number): string {
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  const secs = seconds % 60;
  if (minutes < 60) return secs > 0 ? `${minutes}m ${secs}s` : `${minutes}m`;
  const hours = Math.floor(minutes / 60);
  const mins = minutes % 60;
  return mins > 0 ? `${hours}h ${mins}m` : `${hours}h`;
}

function formatTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`;
  return n.toString();
}

const PAUSE_REASON_KEYS: Record<string, string> = {
  user_pause: "goal_pauseReason_userPause",
  user_interrupt: "goal_pauseReason_userInterrupt",
  max_rounds: "goal_pauseReason_maxRounds",
  budget_exhausted: "goal_pauseReason_budgetExhausted",
  session_reconnect: "goal_pauseReason_sessionReconnect",
};

function useStatusConfig(status: string) {
  const { t } = useTranslation("chat");
  return useMemo(() => {
    const configs: Record<
      string,
      { color: string; bg: string; label: string; icon: typeof Crosshair }
    > = {
      active: {
        color: "var(--green, #48BB78)",
        bg: "color-mix(in srgb, var(--green, #48BB78) 12%, transparent)",
        label: t("goal_status_active"),
        icon: Play,
      },
      paused: {
        color: "var(--yellow, #ECC94B)",
        bg: "color-mix(in srgb, var(--yellow, #ECC94B) 12%, transparent)",
        label: t("goal_status_paused"),
        icon: Pause,
      },
      completed: {
        color: "var(--tint, #4299E1)",
        bg: "color-mix(in srgb, var(--tint, #4299E1) 12%, transparent)",
        label: t("goal_status_completed"),
        icon: Check,
      },
      failed: {
        color: "var(--red, #F56565)",
        bg: "color-mix(in srgb, var(--red, #F56565) 12%, transparent)",
        label: t("goal_status_failed"),
        icon: WarningCircle,
      },
      budget_limited: {
        color: "var(--orange, #ED8936)",
        bg: "color-mix(in srgb, var(--orange, #ED8936) 12%, transparent)",
        label: t("goal_status_budget_limited"),
        icon: WarningCircle,
      },
      cancelled: {
        color: "var(--fill-tertiary)",
        bg: "color-mix(in srgb, var(--fill-tertiary) 10%, transparent)",
        label: t("goal_status_cancelled"),
        icon: X,
      },
    };
    return configs[status] ?? configs.active;
  }, [status, t]);
}

function GoalDetail({ sessionId, goal }: { sessionId: string; goal: GoalData }) {
  const { t } = useTranslation("chat");
  const statusCfg = useStatusConfig(goal.status);
  const [editing, setEditing] = useState(false);
  const [editText, setEditText] = useState(goal.description);
  const [addingBudget, setAddingBudget] = useState(false);
  const [budgetInput, setBudgetInput] = useState("");
  const editRef = useRef<HTMLTextAreaElement>(null);

  useEffect(() => {
    if (editing && editRef.current) {
      editRef.current.focus();
      editRef.current.select();
    }
  }, [editing]);

  const tokenPct = goal.token_budget
    ? Math.min(100, Math.round((goal.tokens_used / goal.token_budget) * 100))
    : null;

  const isTerminal = ["completed", "failed", "cancelled"].includes(goal.status);
  const canPause = goal.status === "active";
  const canResume = goal.status === "paused" || goal.status === "budget_limited";
  const canEdit = !isTerminal;

  const handlePause = useCallback(() => {
    transport.pauseGoal(sessionId).catch(() => {});
  }, [sessionId]);

  const handleResume = useCallback(() => {
    transport.resumeGoal(sessionId).catch(() => {});
  }, [sessionId]);

  const handleClear = useCallback(() => {
    transport.clearGoal(sessionId).catch(() => {});
  }, [sessionId]);

  const handleSaveEdit = useCallback(() => {
    const trimmed = editText.trim();
    if (trimmed && trimmed !== goal.description) {
      transport.editGoal(sessionId, trimmed).catch(() => {});
    }
    setEditing(false);
  }, [sessionId, editText, goal.description]);

  const handleAddBudget = useCallback(() => {
    const amount = parseInt(budgetInput, 10);
    if (amount > 0) {
      transport.addGoalBudget(sessionId, amount).catch(() => {});
      setBudgetInput("");
      setAddingBudget(false);
    }
  }, [sessionId, budgetInput]);

  const StatusIcon = statusCfg.icon;

  return (
    <div className="flex flex-col gap-4 p-4">
      {/* Status header */}
      <div className="flex items-center gap-2">
        <StatusIcon size={16} weight="bold" style={{ color: statusCfg.color }} />
        <span
          className="rounded px-2 py-0.5 text-[11px] font-semibold"
          style={{ color: statusCfg.color, background: statusCfg.bg }}
        >
          {statusCfg.label}
        </span>
        {goal.time_used_seconds > 0 && (
          <span
            className="text-[11px] tabular-nums"
            style={{ color: "var(--fill-quaternary)" }}
          >
            {formatElapsed(goal.time_used_seconds)}
          </span>
        )}
        {goal.continuation_rounds > 0 && (
          <span
            className="text-[10px] tabular-nums"
            style={{ color: "var(--fill-quaternary)" }}
          >
            {t("goal_round", { round: goal.continuation_rounds })}
          </span>
        )}
      </div>

      {/* Pause reason */}
      {goal.pause_reason && (
        <div
          className="flex items-center gap-1.5 rounded-md px-2.5 py-1.5 text-[11px]"
          style={{
            background: "color-mix(in srgb, var(--yellow, #ECC94B) 8%, transparent)",
            color: "var(--fill-secondary)",
          }}
        >
          <WarningCircle size={12} style={{ color: "var(--yellow, #ECC94B)" }} />
          {goal.pause_reason && PAUSE_REASON_KEYS[goal.pause_reason]
            ? t(PAUSE_REASON_KEYS[goal.pause_reason])
            : goal.pause_reason}
        </div>
      )}

      {/* Objective */}
      <div>
        <div className="mb-1 flex items-center gap-1.5">
          <span
            className="text-[10px] font-semibold uppercase tracking-wider"
            style={{ color: "var(--fill-tertiary)" }}
          >
            {t("goal_label")}
          </span>
          {canEdit && !editing && (
            <button
              type="button"
              onClick={() => {
                setEditText(goal.description);
                setEditing(true);
              }}
              className="rounded p-0.5 transition-colors hover:bg-[var(--bg-hover)]"
              style={{ color: "var(--fill-quaternary)" }}
            >
              <PencilSimple size={10} />
            </button>
          )}
        </div>
        {editing ? (
          <div className="flex flex-col gap-1.5">
            <textarea
              ref={editRef}
              value={editText}
              onChange={(e) => setEditText(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter" && !e.shiftKey) {
                  e.preventDefault();
                  handleSaveEdit();
                }
                if (e.key === "Escape") setEditing(false);
              }}
              className="w-full resize-none rounded-md border px-2 py-1.5 text-[12px] leading-relaxed outline-none"
              style={{
                background: "var(--bg-primary)",
                borderColor: "var(--separator)",
                color: "var(--fill-primary)",
              }}
              rows={3}
            />
            <div className="flex gap-1.5">
              <button
                type="button"
                onClick={handleSaveEdit}
                className="flex items-center gap-1 rounded-md px-2 py-1 text-[11px] font-medium"
                style={{
                  background: "var(--tint, #4299E1)",
                  color: "white",
                }}
              >
                <Check size={11} />
                {t("save", { ns: "common" })}
              </button>
              <button
                type="button"
                onClick={() => setEditing(false)}
                className="rounded-md px-2 py-1 text-[11px] transition-colors hover:bg-[var(--bg-hover)]"
                style={{ color: "var(--fill-tertiary)" }}
              >
                {t("cancel", { ns: "common" })}
              </button>
            </div>
          </div>
        ) : (
          <p
            className="text-[12px] leading-relaxed"
            style={{ color: "var(--fill-primary)" }}
          >
            {goal.description}
          </p>
        )}
      </div>

      {/* Token budget */}
      <div>
        <span
          className="mb-1 block text-[10px] font-semibold uppercase tracking-wider"
          style={{ color: "var(--fill-tertiary)" }}
        >
          {t("goal_tokenBudget")}
        </span>
        {goal.token_budget != null && goal.token_budget > 0 ? (
          <div className="flex flex-col gap-1.5">
            <div className="flex items-center gap-2">
              <div
                className="h-[5px] flex-1 overflow-hidden rounded-full"
                style={{ background: "var(--bg-tertiary, rgba(0,0,0,0.06))" }}
              >
                <div
                  className="h-full transition-all duration-300"
                  style={{
                    width: `${tokenPct ?? 0}%`,
                    background:
                      goal.status === "budget_limited"
                        ? "var(--orange, #ED8936)"
                        : "var(--tint, #4299E1)",
                  }}
                />
              </div>
              <span
                className="shrink-0 text-[11px] font-medium tabular-nums"
                style={{ color: "var(--fill-secondary)" }}
              >
                {formatTokens(goal.tokens_used)} / {formatTokens(goal.token_budget)}
              </span>
            </div>
            {tokenPct != null && (
              <span className="text-[10px] tabular-nums" style={{ color: "var(--fill-quaternary)" }}>
                {t("goal_tokensRemaining", {
                  remaining: formatTokens(goal.token_budget - goal.tokens_used),
                  pct: 100 - tokenPct,
                })}
              </span>
            )}
          </div>
        ) : (
          <span className="text-[11px]" style={{ color: "var(--fill-quaternary)" }}>
            {t("goal_unlimitedUsage", { used: formatTokens(goal.tokens_used) })}
          </span>
        )}

        {/* Budget add button */}
        {(goal.status === "budget_limited" || goal.status === "paused") && (
          <div className="mt-2">
            {addingBudget ? (
              <div className="flex items-center gap-1.5">
                <input
                  type="number"
                  value={budgetInput}
                  onChange={(e) => setBudgetInput(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") handleAddBudget();
                    if (e.key === "Escape") setAddingBudget(false);
                  }}
                  placeholder={t("goal_addBudgetPlaceholder")}
                  className="w-24 rounded-md border px-2 py-1 text-[11px] outline-none"
                  style={{
                    background: "var(--bg-primary)",
                    borderColor: "var(--separator)",
                    color: "var(--fill-primary)",
                  }}
                  autoFocus
                />
                <button
                  type="button"
                  onClick={handleAddBudget}
                  className="rounded-md px-2 py-1 text-[11px] font-medium"
                  style={{ background: "var(--tint, #4299E1)", color: "white" }}
                >
                  {t("goal_addBudget")}
                </button>
                <button
                  type="button"
                  onClick={() => setAddingBudget(false)}
                  className="rounded-md px-2 py-1 text-[11px]"
                  style={{ color: "var(--fill-tertiary)" }}
                >
                  {t("cancel", { ns: "common" })}
                </button>
              </div>
            ) : (
              <button
                type="button"
                onClick={() => setAddingBudget(true)}
                className="flex items-center gap-1 rounded-md px-2 py-1 text-[11px] font-medium transition-colors hover:bg-[var(--bg-hover)]"
                style={{ color: "var(--tint, #4299E1)" }}
              >
                <Plus size={11} />
                {t("goal_addBudgetButton")}
              </button>
            )}
          </div>
        )}
      </div>

      {/* Ending conditions */}
      <div>
        <span
          className="mb-1.5 block text-[10px] font-semibold uppercase tracking-wider"
          style={{ color: "var(--fill-tertiary)" }}
        >
          {t("goal_endingConditions")}
        </span>
        <ul className="flex flex-col gap-1 text-[11px]" style={{ color: "var(--fill-secondary)" }}>
          <li>• {t("goal_endCondition_modelCall")}</li>
          {goal.token_budget != null && goal.token_budget > 0 && (
            <li>• {t("goal_endCondition_budgetExhausted", { budget: formatTokens(goal.token_budget) })}</li>
          )}
          <li>• {t("goal_endCondition_maxRounds")}</li>
          <li>• {t("goal_endCondition_userPause")}</li>
        </ul>
      </div>

      {/* Action buttons */}
      {!isTerminal && (
        <div
          className="flex gap-2 border-t pt-3"
          style={{ borderColor: "var(--separator)" }}
        >
          {canPause && (
            <button
              type="button"
              onClick={handlePause}
              className="flex items-center gap-1.5 rounded-md px-3 py-1.5 text-[11px] font-medium transition-colors hover:bg-[var(--bg-hover)]"
              style={{ color: "var(--fill-secondary)" }}
            >
              <Pause size={12} />
              {t("goal_pause")}
            </button>
          )}
          {canResume && (
            <button
              type="button"
              onClick={handleResume}
              className="flex items-center gap-1.5 rounded-md px-3 py-1.5 text-[11px] font-medium"
              style={{
                background: "var(--tint, #4299E1)",
                color: "white",
              }}
            >
              <Play size={12} />
              {t("goal_resume")}
            </button>
          )}
          <div className="flex-1" />
          <button
            type="button"
            onClick={handleClear}
            className="flex items-center gap-1.5 rounded-md px-3 py-1.5 text-[11px] font-medium transition-colors hover:bg-[var(--bg-hover)]"
            style={{ color: "var(--red, #F56565)" }}
          >
            <X size={12} />
            {t("goal_cancelGoal")}
          </button>
        </div>
      )}
    </div>
  );
}

export function GoalTabContent() {
  const { t } = useTranslation("chat");
  const activeChatId = useChatMetaStore((s) => s.activeChatId);
  const goal = useGoalStore((s) => (activeChatId ? s.goals[activeChatId] : undefined));

  if (!goal || !activeChatId) {
    return (
      <div
        className="flex flex-1 flex-col items-center justify-center gap-2 p-6"
        style={{ color: "var(--fill-quaternary)" }}
      >
        <Crosshair size={32} weight="light" />
        <span className="text-[12px]">{t("goal_noActiveGoal")}</span>
        <span className="text-[11px] opacity-60">
          {t("goal_createHint")}
        </span>
      </div>
    );
  }

  return (
    <div className="flex-1 overflow-y-auto">
      <GoalDetail sessionId={activeChatId} goal={goal} />
    </div>
  );
}
