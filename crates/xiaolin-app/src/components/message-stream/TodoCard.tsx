/**
 * Specialized renderer for todo_write tool results.
 * Parses the formatted text output and renders a visual task list.
 */

import { CircleDot, CheckCircle2, Circle, XCircle } from "lucide-react";
import { ICON } from "../../lib/ui-tokens";

interface TodoItem {
  marker: string;
  id: string;
  content: string;
  status: "completed" | "in_progress" | "pending" | "cancelled";
}

export interface TodoSummary {
  total: number;
  completed: number;
  inProgress: number;
  pending: number;
  cancelled: number;
}

export function parseTodoResult(text: string): { summary: TodoSummary; items: TodoItem[] } | null {
  const lines = text.split("\n").filter(Boolean);
  if (lines.length < 2) return null;

  const summaryMatch = lines[0].match(
    /(\d+)\s*total.*?(\d+)\s*completed.*?(\d+)\s*in_progress.*?(\d+)\s*pending.*?(\d+)\s*cancelled/,
  );
  if (!summaryMatch) return null;

  const summary: TodoSummary = {
    total: parseInt(summaryMatch[1]),
    completed: parseInt(summaryMatch[2]),
    inProgress: parseInt(summaryMatch[3]),
    pending: parseInt(summaryMatch[4]),
    cancelled: parseInt(summaryMatch[5]),
  };

  const items: TodoItem[] = [];
  for (let i = 1; i < lines.length; i++) {
    const line = lines[i].trim();
    const match = line.match(/^\[([ x>\-])\]\s+(\S+)\s+—\s+(.+)$/);
    if (!match) continue;
    const [, marker, id, content] = match;
    let status: TodoItem["status"] = "pending";
    if (marker === "x") status = "completed";
    else if (marker === ">") status = "in_progress";
    else if (marker === "-") status = "cancelled";
    items.push({ marker, id, content, status });
  }

  return items.length > 0 ? { summary, items } : null;
}

const STATUS_CONFIG = {
  completed: {
    icon: CheckCircle2,
    color: "var(--green, #48BB78)",
    bg: "color-mix(in srgb, var(--green, #48BB78) 8%, transparent)",
    label: "已完成",
  },
  in_progress: {
    icon: CircleDot,
    color: "var(--tint, #4299E1)",
    bg: "color-mix(in srgb, var(--tint, #4299E1) 8%, transparent)",
    label: "进行中",
  },
  pending: {
    icon: Circle,
    color: "var(--fill-tertiary)",
    bg: "transparent",
    label: "待开始",
  },
  cancelled: {
    icon: XCircle,
    color: "var(--fill-quaternary)",
    bg: "transparent",
    label: "已取消",
  },
} as const;

function ProgressBar({ summary }: { summary: TodoSummary }) {
  if (summary.total === 0) return null;
  const completedPct = (summary.completed / summary.total) * 100;
  const inProgressPct = (summary.inProgress / summary.total) * 100;

  return (
    <div className="flex items-center gap-2">
      <div
        className="h-[4px] flex-1 overflow-hidden rounded-full"
        style={{ background: "var(--bg-tertiary, rgba(0,0,0,0.06))" }}
      >
        <div className="flex h-full">
          <div
            className="h-full transition-all duration-300"
            style={{ width: `${completedPct}%`, background: "var(--green, #48BB78)" }}
          />
          <div
            className="h-full transition-all duration-300"
            style={{ width: `${inProgressPct}%`, background: "var(--tint, #4299E1)" }}
          />
        </div>
      </div>
      <span
        className="shrink-0 text-[10px] font-medium tabular-nums"
        style={{ color: "var(--fill-tertiary)" }}
      >
        {summary.completed}/{summary.total}
      </span>
    </div>
  );
}

export function TodoCard({ result }: { result: string }) {
  const parsed = parseTodoResult(result);
  if (!parsed) return null;

  const { summary, items } = parsed;

  return (
    <div
      className="mt-1.5 overflow-hidden rounded-lg"
      style={{
        border: "0.5px solid var(--separator)",
        background: "var(--bg-secondary)",
      }}
    >
      <div className="px-3 py-2">
        <div className="mb-2 flex items-center justify-between">
          <span
            className="text-[11px] font-semibold uppercase tracking-wider"
            style={{ color: "var(--fill-tertiary)" }}
          >
            任务列表
          </span>
          <div className="flex gap-2 text-[10px]" style={{ color: "var(--fill-quaternary)" }}>
            {summary.inProgress > 0 && (
              <span style={{ color: "var(--tint, #4299E1)" }}>{summary.inProgress} 进行中</span>
            )}
            {summary.pending > 0 && <span>{summary.pending} 待开始</span>}
          </div>
        </div>

        <ProgressBar summary={summary} />

        <div className="mt-2 space-y-0.5">
          {items.map((item) => {
            const cfg = STATUS_CONFIG[item.status];
            const Icon = cfg.icon;
            return (
              <div
                key={item.id}
                className="flex items-start gap-2 rounded-md px-2 py-1.5 transition-colors"
                style={{ background: cfg.bg }}
              >
                <Icon
                  {...ICON.sm}
                  className="mt-[1px] shrink-0"
                  style={{ color: cfg.color, animation: item.status === "completed" ? "scale-spring var(--duration-normal) var(--ease-spring)" : "none" }}
                />
                <div className="min-w-0 flex-1">
                  <span
                    className={`text-[12px] leading-[1.5] ${
                      item.status === "completed" ? "line-through" : ""
                    } ${item.status === "cancelled" ? "line-through opacity-50" : ""}`}
                    style={{
                      color:
                        item.status === "completed" || item.status === "cancelled"
                          ? "var(--fill-tertiary)"
                          : "var(--fill-primary)",
                    }}
                  >
                    {item.content}
                  </span>
                </div>
                <span
                  className="mt-[1px] shrink-0 text-[10px] font-mono"
                  style={{ color: "var(--fill-quaternary)" }}
                >
                  {item.id}
                </span>
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
}

export function isTodoResult(toolName: string, result: string): boolean {
  return (
    toolName === "todo_write" &&
    /Todos:\s*\d+\s*total/.test(result)
  );
}
