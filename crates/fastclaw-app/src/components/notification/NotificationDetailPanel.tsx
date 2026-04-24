import { X, Clock, AlertTriangle, Info, Zap } from "lucide-react";
import type { AppNotification } from "../../lib/transport";
import type { ReactNode } from "react";

function categoryLabel(category: string): { icon: ReactNode; label: string } {
  switch (category) {
    case "cron":
      return { icon: <Clock size={14} />, label: "定时任务" };
    case "agent":
      return { icon: <Zap size={14} />, label: "Agent" };
    case "error":
      return { icon: <AlertTriangle size={14} />, label: "错误" };
    default:
      return { icon: <Info size={14} />, label: "系统" };
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
                {label} · {new Date(notification.createdAt).toLocaleString()}
              </div>
            </div>
          </div>
          <button
            onClick={onClose}
            className="flex items-center justify-center rounded-md p-1.5 hover:bg-[var(--bg-hover)] transition-colors"
            style={{ color: "var(--fill-tertiary)" }}
          >
            <X size={16} />
          </button>
        </div>

        {/* Body */}
        <div className="flex-1 overflow-y-auto px-5 py-4">
          {notification.body && (
            <p
              className="text-[13px] leading-relaxed mb-3"
              style={{ color: "var(--fill-primary)" }}
            >
              {notification.body}
            </p>
          )}

          {notification.detail && (
            <pre
              className="rounded-lg p-3 text-[11px] leading-relaxed whitespace-pre-wrap break-words"
              style={{
                background: "var(--bg-secondary)",
                color: "var(--fill-secondary)",
                border: "0.5px solid var(--separator)",
              }}
            >
              {notification.detail}
            </pre>
          )}
        </div>
      </div>
    </div>
  );
}
