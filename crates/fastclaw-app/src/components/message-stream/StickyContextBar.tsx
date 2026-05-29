import { memo, useCallback, useState } from "react";
import { Square, Pencil, RotateCw, ChevronDown, ChevronUp } from "lucide-react";
import { ICON } from "../../lib/ui-tokens";
import type { TodoSummary } from "./TodoCard";

export interface StickyContextBarProps {
  userMessage: string;
  streaming: boolean;
  todoProgress: TodoSummary | null;
  onStop: () => void;
  onEdit: () => void;
  onResend: () => void;
}

export const StickyContextBar = memo(function StickyContextBar({
  userMessage,
  streaming,
  todoProgress,
  onStop,
  onEdit,
  onResend,
}: StickyContextBarProps) {
  const [expanded, setExpanded] = useState(false);
  const truncated = userMessage.length > 80;
  const displayText = expanded ? userMessage : userMessage.slice(0, 80) + (truncated ? "…" : "");

  const handleStop = useCallback((e: React.MouseEvent) => {
    e.stopPropagation();
    onStop();
  }, [onStop]);

  const handleEdit = useCallback((e: React.MouseEvent) => {
    e.stopPropagation();
    onEdit();
  }, [onEdit]);

  const handleResend = useCallback((e: React.MouseEvent) => {
    e.stopPropagation();
    onResend();
  }, [onResend]);

  const progressPct = todoProgress
    ? Math.round((todoProgress.completed / Math.max(todoProgress.total, 1)) * 100)
    : 0;

  return (
    <div
      className="group/ctx flex shrink-0 items-start gap-2 px-4 py-2"
      style={{
        background: "var(--bg-secondary)",
        borderBottom: "0.5px solid var(--separator)",
        animation: "fade-in var(--duration-fast) var(--ease-out)",
      }}
    >
      <div className="flex min-w-0 flex-1 flex-col gap-1">
        <div className="flex items-center gap-2">
          <span
            className="shrink-0 text-[10px] font-semibold uppercase tracking-wider"
            style={{ color: "var(--fill-tertiary)" }}
          >
            You
          </span>
          <span
            className="min-w-0 truncate text-[13px] leading-snug"
            style={{ color: "var(--fill-primary)" }}
          >
            {displayText}
          </span>
          {truncated && (
            <button
              onClick={() => setExpanded(!expanded)}
              className="flex h-5 w-5 shrink-0 items-center justify-center rounded transition-colors duration-100 hover:bg-[var(--bg-hover)]"
              style={{ color: "var(--fill-tertiary)" }}
            >
              {expanded ? <ChevronUp size={12} strokeWidth={1.5} /> : <ChevronDown size={12} strokeWidth={1.5} />}
            </button>
          )}
        </div>

        {todoProgress && todoProgress.total > 0 && (
          <div className="flex items-center gap-2">
            <div
              className="h-[3px] flex-1 overflow-hidden rounded-full"
              style={{ background: "var(--separator)", maxWidth: 120 }}
            >
              <div
                className="h-full rounded-full transition-all duration-300"
                style={{
                  width: `${progressPct}%`,
                  background: progressPct === 100 ? "var(--green)" : "var(--tint)",
                }}
              />
            </div>
            <span className="text-[10px] tabular-nums" style={{ color: "var(--fill-tertiary)" }}>
              {todoProgress.completed}/{todoProgress.total}
            </span>
          </div>
        )}
      </div>

      <div className="flex shrink-0 items-center gap-0.5 opacity-0 transition-opacity duration-150 group-hover/ctx:opacity-100">
        {streaming && (
          <button
            onClick={handleStop}
            className="flex h-6 items-center gap-1 rounded-md px-1.5 transition-colors duration-100 hover:bg-[var(--bg-hover)]"
            style={{ color: "var(--red)" }}
            title="停止"
          >
            <Square size={10} strokeWidth={2} fill="currentColor" />
            <span className="text-[11px] font-medium">停止</span>
          </button>
        )}
        <button
          onClick={handleEdit}
          className="flex h-6 w-6 items-center justify-center rounded-md transition-colors duration-100 hover:bg-[var(--bg-hover)]"
          style={{ color: "var(--fill-tertiary)" }}
          title="编辑"
        >
          <Pencil {...ICON.sm} />
        </button>
        {!streaming && (
          <button
            onClick={handleResend}
            className="flex h-6 w-6 items-center justify-center rounded-md transition-colors duration-100 hover:bg-[var(--bg-hover)]"
            style={{ color: "var(--fill-tertiary)" }}
            title="重新发送"
          >
            <RotateCw {...ICON.sm} />
          </button>
        )}
      </div>
    </div>
  );
});
