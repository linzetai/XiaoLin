import { X, Clock, AlertTriangle, Info, Zap } from "lucide-react";
import { ICON } from "../../lib/ui-tokens";
import type { AppNotification } from "../../lib/transport";
import { lazy, Suspense, type ReactNode } from "react";

const MarkdownContent = lazy(() =>
  import("../message-stream/MarkdownContent").then((m) => ({ default: m.MarkdownContent })),
);

function parseUtc(ts: string): Date {
  if (!ts) return new Date();
  if (ts.endsWith("Z") || /[+-]\d{2}:\d{2}$/.test(ts)) return new Date(ts);
  return new Date(ts.replace(" ", "T") + "Z");
}

function categoryLabel(category?: string): { icon: ReactNode; label: string } {
  switch (category) {
    case "cron":
      return { icon: <Clock {...ICON.md} />, label: "定时任务" };
    case "agent":
      return { icon: <Zap {...ICON.md} />, label: "Agent" };
    case "error":
      return { icon: <AlertTriangle {...ICON.md} />, label: "错误" };
    default:
      return { icon: <Info {...ICON.md} />, label: "系统" };
  }
}

interface Props {
  notification: AppNotification;
  onClose: () => void;
}

export function NotificationDetailPanel({ notification, onClose }: Props) {
  const { icon, label } = categoryLabel(notification.category);

  return (
    <div
      className="fixed inset-0 z-[70] flex items-center justify-center"
      style={{ background: "rgba(0,0,0,0.4)" }}
      onClick={onClose}
    >
      <div
        className="relative flex flex-col rounded-xl overflow-hidden"
        style={{
          width: "420px",
          maxHeight: "70vh",
          background: "var(--bg-primary)",
          border: "0.5px solid var(--separator)",
          boxShadow: "0 12px 40px rgba(0,0,0,0.3)",
        }}
        onClick={(e) => e.stopPropagation()}
      >
        {/* Header */}
        <div
          className="flex items-center justify-between px-5 py-3"
          style={{ borderBottom: "0.5px solid var(--separator)" }}
        >
          <div className="flex items-center gap-2">
            <div
              className="flex items-center justify-center rounded-md"
              style={{
                width: "28px",
                height: "28px",
                background: "var(--bg-secondary)",
                color: "var(--fill-secondary)",
              }}
            >
              {icon}
            </div>
            <div>
              <div
                className="text-[13px] font-semibold"
                style={{ color: "var(--fill-primary)" }}
              >
                {notification.title}
              </div>
              <div
                className="text-[10px]"
                style={{ color: "var(--fill-quaternary)" }}
              >
                {label} · {parseUtc(notification.createdAt).toLocaleString()}
              </div>
            </div>
          </div>
          <button
            onClick={onClose}
            className="flex items-center justify-center rounded-md p-1.5 hover:bg-[var(--bg-hover)] transition-colors"
            style={{ color: "var(--fill-tertiary)" }}
          >
            <X {...ICON.md} />
          </button>
        </div>

        {/* Body */}
        <div className="flex-1 overflow-y-auto px-5 py-4">
          {notification.body && (
            <div
              className="text-[13px] leading-relaxed mb-3"
              style={{ color: "var(--fill-primary)" }}
            >
              <Suspense fallback={<div className="animate-pulse rounded py-1" style={{ background: "var(--bg-tertiary)", height: 16 }} />}>
                <MarkdownContent content={notification.body} />
              </Suspense>
            </div>
          )}

          {notification.detail && (
            <div
              className="rounded-lg p-3 text-[11px] leading-relaxed"
              style={{
                background: "var(--bg-secondary)",
                color: "var(--fill-secondary)",
                border: "0.5px solid var(--separator)",
              }}
            >
              <Suspense fallback={<div className="animate-pulse rounded py-1" style={{ background: "var(--bg-tertiary)", height: 16 }} />}>
                <MarkdownContent content={notification.detail} />
              </Suspense>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
