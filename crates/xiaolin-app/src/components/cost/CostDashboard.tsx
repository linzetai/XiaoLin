import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { ArrowsClockwise, CheckCircle, XCircle, Lightning, Clock, CurrencyDollar, ChatText } from "@phosphor-icons/react";
import { useCostStore, type TokenUsageDaily, type ToolCallDaily, type SessionCostSummary } from "../../lib/stores/cost-store";

function formatUsd(v: number): string {
  if (v < 0.01) return `$${v.toFixed(4)}`;
  if (v < 1) return `$${v.toFixed(3)}`;
  return `$${v.toFixed(2)}`;
}

function formatTokens(v: number): string {
  if (v >= 1_000_000) return `${(v / 1_000_000).toFixed(1)}M`;
  if (v >= 1_000) return `${(v / 1_000).toFixed(1)}K`;
  return String(v);
}

function StatCard({ icon, label, value, sub }: { icon: React.ReactNode; label: string; value: string; sub?: string }) {
  return (
    <div
      className="flex flex-col gap-1.5 rounded-[var(--radius-sm)] px-4 py-3"
      style={{ background: "var(--bg-elevated)", border: "0.5px solid var(--separator-opaque)" }}
    >
      <div className="flex items-center gap-1.5">
        <span style={{ color: "var(--fill-quaternary)" }}>{icon}</span>
        <span className="text-[11px]" style={{ color: "var(--fill-tertiary)" }}>{label}</span>
      </div>
      <div className="flex items-baseline gap-1.5">
        <span className="text-[18px] font-bold tabular-nums" style={{ color: "var(--fill-primary)" }}>{value}</span>
        {sub && <span className="text-[11px]" style={{ color: "var(--fill-quaternary)" }}>{sub}</span>}
      </div>
    </div>
  );
}

function SectionTitle({ children, action }: { children: React.ReactNode; action?: React.ReactNode }) {
  return (
    <div className="mb-3 flex items-center justify-between">
      <h3 className="text-[12px] font-semibold" style={{ color: "var(--fill-secondary)" }}>
        {children}
      </h3>
      {action}
    </div>
  );
}

function TrendChart({ data }: { data: TokenUsageDaily[] }) {
  const { t } = useTranslation("cost");
  const grouped = useMemo(() => {
    const map = new Map<string, { input: number; output: number; cache: number; cost: number }>();
    for (const row of data) {
      const prev = map.get(row.date) ?? { input: 0, output: 0, cache: 0, cost: 0 };
      prev.input += row.input_tokens;
      prev.output += row.output_tokens;
      prev.cache += row.cache_read_tokens;
      prev.cost += row.cost_usd;
      map.set(row.date, prev);
    }
    return Array.from(map.entries())
      .sort(([a], [b]) => a.localeCompare(b))
      .slice(-14);
  }, [data]);

  if (grouped.length === 0) {
    return (
      <div className="px-4 py-8 text-center text-[12px]" style={{ color: "var(--fill-quaternary)" }}>
        {t("costNoUsageData")}
      </div>
    );
  }

  const maxTokens = Math.max(...grouped.map(([, v]) => v.input + v.output + v.cache), 1);

  return (
    <div className="px-4 py-4">
      <div className="mb-3 flex items-center gap-4 text-[10px]" style={{ color: "var(--fill-tertiary)" }}>
        <span className="flex items-center gap-1"><span className="inline-block h-2 w-2 rounded-[1px]" style={{ background: "var(--tint)" }} />{t("costInput")}</span>
        <span className="flex items-center gap-1"><span className="inline-block h-2 w-2 rounded-[1px]" style={{ background: "var(--green)" }} />{t("costOutput")}</span>
        <span className="flex items-center gap-1"><span className="inline-block h-2 w-2 rounded-[1px]" style={{ background: "var(--fill-quaternary)", opacity: 0.4 }} />{t("costCacheHit")}</span>
      </div>
      <div className="relative flex items-end gap-[4px]" style={{ height: 100 }}>
        {grouped.map(([date, v]) => {
          const total = v.input + v.output + v.cache;
          const pct = total / maxTokens;
          const inputPct = total > 0 ? v.input / total : 0;
          const outputPct = total > 0 ? v.output / total : 0;
          const barH = Math.max(4, pct * 100);
          return (
            <div key={date} className="group relative flex flex-1 flex-col items-center justify-end" style={{ height: "100%" }}>
              <div className="flex w-full flex-col rounded-[2px] overflow-hidden" style={{ height: `${barH}%` }}>
                <div style={{ flex: `0 0 ${outputPct * 100}%`, background: "var(--green)" }} />
                <div style={{ flex: `0 0 ${inputPct * 100}%`, background: "var(--tint)" }} />
                <div style={{ flex: 1, background: "var(--fill-quaternary)", opacity: 0.25 }} />
              </div>
              <div
                className="pointer-events-none absolute bottom-full z-50 mb-1 hidden rounded-[var(--radius-xs)] px-2.5 py-2 text-[11px] group-hover:block"
                style={{
                  background: "var(--bg-elevated)",
                  border: "0.5px solid var(--separator-opaque)",
                  boxShadow: "var(--shadow-md)",
                  color: "var(--fill-secondary)",
                  whiteSpace: "nowrap",
                }}
              >
                <div className="mb-1 font-medium" style={{ color: "var(--fill-primary)" }}>{date}</div>
                <div>{t("costInputAmount", { amount: formatTokens(v.input) })}</div>
                <div>{t("costOutputAmount", { amount: formatTokens(v.output) })}</div>
                {v.cache > 0 && <div>cache: {formatTokens(v.cache)}</div>}
                <div className="mt-0.5 font-medium">{formatUsd(v.cost)}</div>
              </div>
            </div>
          );
        })}
      </div>
      <div className="mt-2 flex justify-between text-[10px]" style={{ color: "var(--fill-quaternary)" }}>
        <span>{grouped[0]?.[0]?.slice(5)}</span>
        <span>{grouped[grouped.length - 1]?.[0]?.slice(5)}</span>
      </div>
    </div>
  );
}

function TokenComposition({ data }: { data: TokenUsageDaily[] }) {
  const { t } = useTranslation("cost");
  const totals = useMemo(() => {
    let input = 0, output = 0, cache = 0;
    for (const row of data) {
      input += row.input_tokens;
      output += row.output_tokens;
      cache += row.cache_read_tokens;
    }
    return { input, output, cache, total: input + output + cache };
  }, [data]);

  if (totals.total === 0) {
    return (
      <div className="px-4 py-8 text-center text-[12px]" style={{ color: "var(--fill-quaternary)" }}>
        {t("costNoData")}
      </div>
    );
  }

  const inputPct = (totals.input / totals.total * 100).toFixed(1);
  const outputPct = (totals.output / totals.total * 100).toFixed(1);
  const cachePct = (totals.cache / totals.total * 100).toFixed(1);

  return (
    <div className="px-4 py-4">
      <div className="mb-3 text-center">
        <div className="text-[22px] font-bold tabular-nums" style={{ color: "var(--fill-primary)" }}>
          {formatTokens(totals.total)}
        </div>
        <div className="text-[11px]" style={{ color: "var(--fill-quaternary)" }}>{t("costTotalTokens")}</div>
      </div>
      <div className="mb-3 flex h-2 overflow-hidden rounded-full">
        <div style={{ width: `${inputPct}%`, background: "var(--tint)" }} />
        <div style={{ width: `${outputPct}%`, background: "var(--green)" }} />
        <div style={{ width: `${cachePct}%`, background: "var(--fill-quaternary)", opacity: 0.35 }} />
      </div>
      <div className="space-y-1.5 text-[11px]">
        <div className="flex items-center justify-between">
          <span className="flex items-center gap-1.5" style={{ color: "var(--fill-secondary)" }}>
            <span className="inline-block h-2 w-2 rounded-[1px]" style={{ background: "var(--tint)" }} />{t("costInput")}
          </span>
          <span className="tabular-nums" style={{ color: "var(--fill-primary)" }}>{formatTokens(totals.input)}</span>
        </div>
        <div className="flex items-center justify-between">
          <span className="flex items-center gap-1.5" style={{ color: "var(--fill-secondary)" }}>
            <span className="inline-block h-2 w-2 rounded-[1px]" style={{ background: "var(--green)" }} />{t("costOutput")}
          </span>
          <span className="tabular-nums" style={{ color: "var(--fill-primary)" }}>{formatTokens(totals.output)}</span>
        </div>
        <div className="flex items-center justify-between">
          <span className="flex items-center gap-1.5" style={{ color: "var(--fill-secondary)" }}>
            <span className="inline-block h-2 w-2 rounded-[1px]" style={{ background: "var(--fill-quaternary)", opacity: 0.4 }} />{t("costCacheHit")}
          </span>
          <span className="tabular-nums" style={{ color: "var(--fill-primary)" }}>{formatTokens(totals.cache)}</span>
        </div>
      </div>
    </div>
  );
}

function ToolTable({ data }: { data: ToolCallDaily[] }) {
  const { t } = useTranslation("cost");
  const sorted = useMemo(() => {
    const agg = new Map<string, { success: number; failure: number; duration: number }>();
    for (const row of data) {
      const prev = agg.get(row.tool_name) ?? { success: 0, failure: 0, duration: 0 };
      prev.success += row.success_count;
      prev.failure += row.failure_count;
      prev.duration += row.total_duration_ms;
      agg.set(row.tool_name, prev);
    }
    return Array.from(agg.entries())
      .map(([name, v]) => ({ name, ...v, total: v.success + v.failure }))
      .sort((a, b) => b.total - a.total);
  }, [data]);

  if (sorted.length === 0) {
    return (
      <div className="px-4 py-6 text-center text-[12px]" style={{ color: "var(--fill-quaternary)" }}>
        {t("costNoToolData")}
      </div>
    );
  }

  return (
    <div className="overflow-auto" style={{ maxHeight: 200 }}>
      {sorted.slice(0, 15).map((row) => {
        const rate = row.total > 0 ? (row.success / row.total) * 100 : 0;
        const avgMs = row.total > 0 ? row.duration / row.total : 0;
        return (
          <div
            key={row.name}
            className="flex items-center gap-3 px-4 py-2"
            style={{ borderBottom: "0.5px solid var(--separator)" }}
          >
            <span className="min-w-0 flex-1 truncate text-[12px] font-mono" style={{ color: "var(--fill-primary)" }}>
              {row.name}
            </span>
            <span className="flex items-center gap-0.5 text-[11px] tabular-nums" style={{ color: "var(--green)" }}>
              <CheckCircle size={10} /> {row.success}
            </span>
            {row.failure > 0 && (
              <span className="flex items-center gap-0.5 text-[11px] tabular-nums" style={{ color: "var(--red)" }}>
                <XCircle size={10} /> {row.failure}
              </span>
            )}
            <span className="w-10 text-right text-[11px] tabular-nums" style={{ color: rate >= 90 ? "var(--green)" : rate >= 70 ? "var(--yellow)" : "var(--red)" }}>
              {rate.toFixed(0)}%
            </span>
            <span className="w-12 text-right text-[11px] tabular-nums" style={{ color: "var(--fill-quaternary)" }}>
              {avgMs.toFixed(0)}ms
            </span>
          </div>
        );
      })}
    </div>
  );
}

function SessionTable({ data }: { data: SessionCostSummary[] }) {
  const { t } = useTranslation("cost");
  if (data.length === 0) {
    return (
      <div className="px-4 py-6 text-center text-[12px]" style={{ color: "var(--fill-quaternary)" }}>
        {t("costNoSessionData")}
      </div>
    );
  }

  return (
    <div className="overflow-auto text-[11px]" style={{ maxHeight: 220 }}>
      <table className="w-full">
        <thead>
          <tr style={{ borderBottom: "0.5px solid var(--separator-opaque)" }}>
            <th className="px-3 py-2 text-left font-medium" style={{ color: "var(--fill-tertiary)" }}>{t("costDate")}</th>
            <th className="px-3 py-2 text-left font-medium" style={{ color: "var(--fill-tertiary)" }}>{t("costModel")}</th>
            <th className="px-3 py-2 text-right font-medium" style={{ color: "var(--fill-tertiary)" }}>Input</th>
            <th className="px-3 py-2 text-right font-medium" style={{ color: "var(--fill-tertiary)" }}>Output</th>
            <th className="px-3 py-2 text-right font-medium" style={{ color: "var(--fill-tertiary)" }}>{t("costFee")}</th>
            <th className="px-3 py-2 text-right font-medium" style={{ color: "var(--fill-tertiary)" }}>{t("costTurns")}</th>
          </tr>
        </thead>
        <tbody>
          {data.map((row) => (
            <tr key={row.session_id} style={{ borderBottom: "0.5px solid var(--separator)" }}>
              <td className="px-3 py-2 tabular-nums" style={{ color: "var(--fill-secondary)" }}>
                {row.started_at?.slice(0, 16)?.replace("T", " ")}
              </td>
              <td className="px-3 py-2 font-mono" style={{ color: "var(--fill-secondary)" }}>
                {row.model_breakdown?.split("/").pop() ?? "-"}
              </td>
              <td className="px-3 py-2 text-right tabular-nums" style={{ color: "var(--fill-primary)" }}>
                {formatTokens(row.total_input_tokens)}
              </td>
              <td className="px-3 py-2 text-right tabular-nums" style={{ color: "var(--fill-primary)" }}>
                {formatTokens(row.total_output_tokens)}
              </td>
              <td className="px-3 py-2 text-right tabular-nums font-medium" style={{ color: "var(--fill-primary)" }}>
                {formatUsd(row.total_cost_usd)}
              </td>
              <td className="px-3 py-2 text-right tabular-nums" style={{ color: "var(--fill-quaternary)" }}>
                {row.turn_count}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

export function CostDashboard() {
  const { t } = useTranslation("cost");
  const summary = useCostStore((s) => s.summary);
  const dailyTokens = useCostStore((s) => s.dailyTokens);
  const toolStats = useCostStore((s) => s.toolStats);
  const sessions = useCostStore((s) => s.sessions);
  const loading = useCostStore((s) => s.loading);
  const fetchAll = useCostStore((s) => s.fetchAll);
  const [rangeStart] = useState(() => {
    const d = new Date();
    d.setDate(d.getDate() - 30);
    return d.toISOString().slice(0, 10);
  });

  useEffect(() => {
    fetchAll(rangeStart);
  }, [fetchAll, rangeStart]);

  const totalTokens = useMemo(
    () => dailyTokens.reduce((a, r) => a + r.input_tokens + r.output_tokens + r.cache_read_tokens, 0),
    [dailyTokens],
  );
  const sessionCount = sessions.length;

  return (
    <div className="space-y-5">
      {/* Top Header */}
      <div className="flex items-center justify-between">
        <h2 className="text-[13px] font-semibold" style={{ color: "var(--fill-primary)" }}>{t("costOverview")}</h2>
        <button
          onClick={() => fetchAll(rangeStart)}
          disabled={loading}
          className="flex cursor-pointer items-center gap-1 rounded-[var(--radius-xs)] px-2 py-1 text-[11px] font-medium transition-colors duration-100 hover:bg-[var(--bg-hover)]"
          style={{ color: "var(--fill-tertiary)" }}
        >
          <ArrowsClockwise size={12} className={loading ? "animate-spin" : ""} />
          {t("costRefresh")}
        </button>
      </div>

      {/* Stats Cards Row */}
      <div className="grid grid-cols-4 gap-2.5">
        <StatCard icon={<Lightning size={13} />} label={t("costTokenUsage")} value={formatTokens(totalTokens)} />
        <StatCard icon={<CurrencyDollar size={13} />} label={t("costSpend")} value={formatUsd(summary?.total_cost_usd ?? 0)} sub={t("costTodaySpend", { amount: formatUsd(summary?.today_cost_usd ?? 0) })} />
        <StatCard icon={<ChatText size={13} />} label={t("costSessionCount")} value={String(sessionCount)} />
        <StatCard icon={<Clock size={13} />} label={t("costLlmCalls")} value={formatTokens(dailyTokens.reduce((a, r) => a + r.call_count, 0))} sub={t("costTimes")} />
      </div>

      {/* Trend + Composition Row */}
      <div className="grid grid-cols-3 gap-2.5">
        <div
          className="col-span-2 overflow-visible rounded-[var(--radius-sm)]"
          style={{ background: "var(--bg-elevated)", border: "0.5px solid var(--separator-opaque)" }}
        >
          <div className="px-4 pt-3 text-[12px] font-medium" style={{ color: "var(--fill-secondary)" }}>{t("costTrendAnalysis")}</div>
          <TrendChart data={dailyTokens} />
        </div>
        <div
          className="rounded-[var(--radius-sm)]"
          style={{ background: "var(--bg-elevated)", border: "0.5px solid var(--separator-opaque)" }}
        >
          <div className="px-4 pt-3 text-[12px] font-medium" style={{ color: "var(--fill-secondary)" }}>{t("costTokenComposition")}</div>
          <TokenComposition data={dailyTokens} />
        </div>
      </div>

      {/* Tool Stats */}
      <div>
        <SectionTitle>{t("costToolStats")}</SectionTitle>
        <div className="rounded-[var(--radius-sm)]" style={{ background: "var(--bg-elevated)", border: "0.5px solid var(--separator-opaque)" }}>
          <ToolTable data={toolStats} />
        </div>
      </div>

      {/* Session Detail Table */}
      <div>
        <SectionTitle>{t("costUsageDetail")}</SectionTitle>
        <div className="rounded-[var(--radius-sm)]" style={{ background: "var(--bg-elevated)", border: "0.5px solid var(--separator-opaque)" }}>
          <SessionTable data={sessions} />
        </div>
      </div>
    </div>
  );
}
