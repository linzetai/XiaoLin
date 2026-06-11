import { useCallback, useMemo } from "react";
import { useTranslation } from "react-i18next";
import { Crosshair, CaretRight } from "@phosphor-icons/react";
import type { GoalData } from "../../lib/stores/types";
import { useWorkspaceTabs } from "../shell/workspace-tabs";

interface GoalStatusCardProps {
  sessionId: string;
  goal: GoalData;
}

function useStatusConfig(status: string) {
  const { t } = useTranslation("chat");
  return useMemo(() => {
    const configs: Record<string, { color: string; bg: string; label: string }> = {
      active: {
        color: "var(--green, #48BB78)",
        bg: "color-mix(in srgb, var(--green, #48BB78) 12%, transparent)",
        label: t("goal_status_active"),
      },
      paused: {
        color: "var(--yellow, #ECC94B)",
        bg: "color-mix(in srgb, var(--yellow, #ECC94B) 12%, transparent)",
        label: t("goal_status_paused"),
      },
      completed: {
        color: "var(--tint, #4299E1)",
        bg: "color-mix(in srgb, var(--tint, #4299E1) 12%, transparent)",
        label: t("goal_status_completed"),
      },
      failed: {
        color: "var(--red, #F56565)",
        bg: "color-mix(in srgb, var(--red, #F56565) 12%, transparent)",
        label: t("goal_status_failed"),
      },
      budget_limited: {
        color: "var(--orange, #ED8936)",
        bg: "color-mix(in srgb, var(--orange, #ED8936) 12%, transparent)",
        label: t("goal_status_budget_limited"),
      },
      cancelled: {
        color: "var(--fill-tertiary)",
        bg: "color-mix(in srgb, var(--fill-tertiary) 10%, transparent)",
        label: t("goal_status_cancelled"),
      },
    };
    return configs[status] ?? configs.active;
  }, [status, t]);
}

export function GoalStatusCard({ goal }: GoalStatusCardProps) {
  const statusCfg = useStatusConfig(goal.status);
  const setActiveTab = useWorkspaceTabs((s) => s.setActiveTab);

  const tokenPct = goal.token_budget
    ? Math.min(100, Math.round((goal.tokens_used / goal.token_budget) * 100))
    : null;

  const openGoalPanel = useCallback(() => {
    setActiveTab("goal");
  }, [setActiveTab]);

  return (
    <button
      type="button"
      onClick={openGoalPanel}
      className="group flex w-full shrink-0 items-center gap-2.5 transition-colors duration-100 hover:bg-[var(--bg-hover)]"
      style={{
        background: "var(--bg-secondary)",
        borderBottom: "0.5px solid var(--separator)",
        padding: "8px clamp(24px, 5%, 80px)",
        cursor: "pointer",
      }}
    >
      <Crosshair
        size={14}
        style={{ color: statusCfg.color, flexShrink: 0 }}
      />

      <span
        className="min-w-0 flex-1 truncate text-left text-[12px]"
        style={{ color: "var(--fill-primary)" }}
      >
        {goal.description}
      </span>

      <span
        className="shrink-0 rounded px-1.5 py-0.5 text-[10px] font-semibold"
        style={{ color: statusCfg.color, background: statusCfg.bg }}
      >
        {statusCfg.label}
      </span>

      {tokenPct != null && (
        <span
          className="shrink-0 text-[10px] font-medium tabular-nums"
          style={{ color: "var(--fill-tertiary)" }}
        >
          {tokenPct}%
        </span>
      )}

      <CaretRight
        size={12}
        className="shrink-0 opacity-30 transition-opacity group-hover:opacity-60"
      />
    </button>
  );
}
