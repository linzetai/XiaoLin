import { useState, useCallback, useMemo } from "react";
import { useTranslation } from "react-i18next";
import { ShieldWarning, ShieldCheck, ShieldSlash, CaretDown, CaretUp } from "@phosphor-icons/react";
import { usePermissionStore } from "../../lib/stores/permission-store";

export interface ApprovalData {
  approvalId: string;
  reason: string;
  action?: {
    action_type?: string;
    command?: string;
    path?: string;
    content?: string;
    diff?: string;
  };
  decisions: Array<{ id: string; label: string }>;
  riskLevel: "danger" | "caution" | "safe";
}

interface ApprovalCardProps {
  data: ApprovalData;
  onDecision: (decision: string) => void;
  sessionId?: string;
}

export function ApprovalCard({ data, onDecision, sessionId }: ApprovalCardProps) {
  const { t } = useTranslation("chat");
  const riskStyles = useMemo(() => ({
    danger: {
      border: "var(--color-red-400, #fc8181)",
      bg: "var(--color-red-50, rgba(254, 215, 215, 0.15))",
      label: t("approval_riskForbidden"),
      icon: ShieldSlash,
      iconColor: "var(--color-red-500, #f56565)",
    },
    caution: {
      border: "var(--color-amber-400, #f6ad55)",
      bg: "var(--color-amber-50, rgba(254, 235, 200, 0.15))",
      label: t("approval_riskCaution"),
      icon: ShieldWarning,
      iconColor: "var(--color-amber-500, #ed8936)",
    },
    safe: {
      border: "var(--color-green-400, #68d391)",
      bg: "var(--color-green-50, rgba(198, 246, 213, 0.15))",
      label: t("approval_riskSafe"),
      icon: ShieldCheck,
      iconColor: "var(--color-green-500, #48bb78)",
    },
  }), [t]);
  const [submitted, setSubmitted] = useState(false);
  const isFileAction = data.action?.action_type === "write_file" || data.action?.action_type === "edit_file";
  const [expanded, setExpanded] = useState(isFileAction);
  const setSessionPreset = usePermissionStore((s) => s.setSessionPreset);

  const style = riskStyles[data.riskLevel];
  const Icon = style.icon;

  const handleDecision = useCallback((decision: string) => {
    if (submitted) return;
    setSubmitted(true);
    onDecision(decision);
  }, [submitted, onDecision]);

  const handleApproveAllForSession = useCallback(async () => {
    if (submitted || !sessionId) return;
    setSubmitted(true);
    await setSessionPreset(sessionId, "full-auto");
    onDecision("approved");
  }, [submitted, sessionId, setSessionPreset, onDecision]);

  const hasPreview = data.action?.command || data.action?.diff || data.action?.content;
  const isForbidden = data.riskLevel === "danger";

  return (
    <div
      className="mx-3 mb-3 overflow-hidden rounded-xl transition-all duration-200"
      style={{
        border: `1.5px solid ${style.border}`,
        background: style.bg,
      }}
    >
      <div className="flex items-center gap-2.5 px-4 py-3">
        <Icon size={18} style={{ color: style.iconColor, flexShrink: 0 }} />
        <span
          className="text-xs font-semibold uppercase tracking-wide"
          style={{ color: style.iconColor }}
        >
          {style.label}
        </span>
      </div>

      <div className="px-4 pb-3">
        <p className="text-sm leading-relaxed" style={{ color: "var(--fill-secondary)" }}>
          {data.reason}
        </p>
        {data.action?.action_type && (
          <span
            className="mt-1.5 inline-block rounded-md px-2 py-0.5 text-xs font-medium"
            style={{
              background: "var(--bg-elevated, rgba(0,0,0,0.06))",
              color: "var(--fill-tertiary)",
            }}
          >
            {data.action.action_type}
          </span>
        )}
      </div>

      {hasPreview && (
        <div className="border-t px-4 py-2" style={{ borderColor: "var(--separator)" }}>
          <button
            className="flex w-full items-center gap-1.5 text-xs font-medium"
            style={{ color: "var(--fill-tertiary)" }}
            onClick={() => setExpanded(!expanded)}
          >
            {expanded ? <CaretUp size={14} /> : <CaretDown size={14} />}
            {data.action?.command ? t("approval_commandPreview") : isFileAction ? t("approval_fileChangePreview") : t("approval_contentPreview")}
          </button>
          {expanded && (
            <>
              {data.action?.path && (
                <div
                  className="mt-1.5 truncate text-xs"
                  style={{ color: "var(--fill-tertiary)", fontFamily: "var(--font-mono, ui-monospace, monospace)" }}
                  title={data.action.path}
                >
                  {data.action.path}
                </div>
              )}
              <pre
                className="mt-2 max-h-48 overflow-auto rounded-lg p-3 text-xs leading-relaxed"
                style={{
                  background: "var(--bg-code, rgba(0,0,0,0.04))",
                  color: "var(--fill-primary)",
                  fontFamily: "var(--font-mono, ui-monospace, monospace)",
                }}
              >
                {data.action?.command || data.action?.diff || data.action?.content}
              </pre>
            </>
          )}
        </div>
      )}

      <div
        className="flex flex-wrap items-center gap-2 border-t px-4 py-3"
        style={{ borderColor: "var(--separator)" }}
      >
        {data.decisions
          .filter((d) => !isForbidden || d.id === "denied" || d.id === "abort")
          .map((d) => (
            <button
              key={d.id}
              onClick={() => handleDecision(d.id)}
              disabled={submitted}
              className="rounded-lg px-3 py-1.5 text-xs font-medium transition-all duration-150 hover:scale-[1.02] active:scale-95 disabled:opacity-40"
              style={getButtonStyle(d.id)}
            >
              {d.label}
            </button>
          ))}
        {!isForbidden && sessionId && (
          <button
            onClick={handleApproveAllForSession}
            disabled={submitted}
            className="ml-auto flex items-center gap-1 rounded-lg px-3 py-1.5 text-xs font-medium transition-all duration-150 hover:scale-[1.02] active:scale-95 disabled:opacity-40"
            style={{
              background: "var(--color-amber-50, rgba(254, 235, 200, 0.2))",
              color: "var(--color-amber-600, #c05621)",
              border: "1px solid var(--color-amber-200, rgba(237, 137, 54, 0.3))",
            }}
            title={t("approval_approveAllSessionTitle")}
          >
            <ShieldSlash size={12} />
            {t("approval_approveAllSession")}
          </button>
        )}
      </div>
    </div>
  );
}

function getButtonStyle(decision: string): React.CSSProperties {
  switch (decision) {
    case "approved":
      return {
        background: "var(--color-green-500, #48bb78)",
        color: "#fff",
      };
    case "approved_for_session":
      return {
        background: "var(--color-blue-500, #4299e1)",
        color: "#fff",
      };
    case "denied":
      return {
        background: "var(--bg-elevated, rgba(0,0,0,0.06))",
        color: "var(--fill-secondary)",
        border: "1px solid var(--separator)",
      };
    case "abort":
      return {
        background: "var(--color-red-500, #f56565)",
        color: "#fff",
      };
    default:
      return {
        background: "var(--bg-elevated, rgba(0,0,0,0.06))",
        color: "var(--fill-secondary)",
      };
  }
}
