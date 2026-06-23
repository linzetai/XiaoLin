import { useState, useCallback, useEffect, useRef } from "react";
import { useTranslation } from "react-i18next";
import { CaretDown, CaretUp } from "@phosphor-icons/react";
import { usePermissionStore } from "../../lib/stores/permission-store";

export interface ApprovalData {
  approvalId: string;
  reason: string;
  action?: {
    action_type?: string;
    command?: string;
    path?: string;
    paths?: string[];
    cwd?: string;
    content?: string;
    diff?: string;
    server_id?: string;
    tool_name?: string;
    arguments_summary?: string;
  };
  decisions: Array<{ id: string; label: string; prefix?: string[] }>;
  riskLevel: "low" | "medium" | "high";
}

interface ApprovalCardProps {
  data: ApprovalData;
  onDecision: (decision: string, extra?: Record<string, unknown>) => void;
  sessionId?: string;
}

const RISK_COLORS: Record<string, string> = {
  high: "var(--color-red-400, #fc8181)",
  medium: "var(--color-amber-400, #f6ad55)",
  low: "var(--color-gray-300, #a0aec0)",
};

const SHORTCUT_MAP: Record<string, string> = {
  y: "approved",
  s: "approved_for_session",
  p: "approved_with_policy_amend",
  n: "denied",
  a: "abort",
};

const INTENT_TITLE_KEYS: Record<string, string> = {
  shell_command: "approval_intentShellCommand",
  file_write: "approval_intentFileWrite",
  apply_patch: "approval_intentApplyPatch",
  network_access: "approval_intentNetworkAccess",
  mcp_tool_call: "approval_intentMcpToolCall",
};

const DECISION_LABEL_KEYS: Record<string, string> = {
  approved: "decision_approved",
  approved_for_session: "decision_approvedSession",
  denied: "decision_denied",
  abort: "decision_abort",
};

function shortcutForDecision(id: string): string | null {
  for (const [key, val] of Object.entries(SHORTCUT_MAP)) {
    if (val === id) return key;
  }
  return null;
}

export function ApprovalCard({ data, onDecision, sessionId }: ApprovalCardProps) {
  const { t } = useTranslation("chat");
  const [submitted, setSubmitted] = useState<string | null>(null);
  const submittedRef = useRef(false);
  const setSessionPreset = usePermissionStore((s) => s.setSessionPreset);

  const borderColor = RISK_COLORS[data.riskLevel] ?? RISK_COLORS.medium;
  const intentTitleKey = data.action?.action_type
    ? INTENT_TITLE_KEYS[data.action.action_type]
    : undefined;
  const intentTitle = intentTitleKey ? t(intentTitleKey) : t("approval_intentDefault");

  const previewContent = data.action?.content || data.action?.diff;
  const previewLineCount = previewContent ? previewContent.split("\n").length : 0;
  const [previewExpanded, setPreviewExpanded] = useState(previewLineCount > 0 && previewLineCount <= 5);

  const handleDecision = useCallback((decisionId: string) => {
    if (submittedRef.current) return;
    submittedRef.current = true;
    setSubmitted(decisionId);

    const matchedDecision = data.decisions.find((d) => d.id === decisionId);
    if (decisionId === "approved_with_policy_amend" && matchedDecision?.prefix) {
      onDecision(decisionId, { prefix: matchedDecision.prefix });
    } else {
      onDecision(decisionId);
    }
  }, [onDecision, data.decisions]);

  const handleApproveAllForSession = useCallback(async () => {
    if (submittedRef.current || !sessionId) return;
    submittedRef.current = true;
    setSubmitted("approved_all_for_session");
    try {
      await setSessionPreset(sessionId, "full-auto");
      onDecision("approved_all_for_session");
    } catch {
      submittedRef.current = false;
      setSubmitted(null);
    }
  }, [sessionId, setSessionPreset, onDecision]);

  useEffect(() => {
    if (submittedRef.current) return;

    const availableIds = new Set(data.decisions.map((d) => d.id));

    const handler = (e: KeyboardEvent) => {
      if (submittedRef.current) return;
      const target = e.target as HTMLElement;
      if (target.tagName === "INPUT" || target.tagName === "TEXTAREA" || target.isContentEditable) {
        return;
      }
      const mapped = SHORTCUT_MAP[e.key.toLowerCase()];
      if (mapped && availableIds.has(mapped)) {
        e.preventDefault();
        handleDecision(mapped);
      }
    };

    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [data.decisions, handleDecision]);

  return (
    <div
      className="mx-3 mb-3 py-3 pr-4 pl-4"
      style={{ borderLeft: `3px solid ${borderColor}` }}
    >
      {/* Intent title */}
      <div className="mb-2 text-sm font-medium" style={{ color: "var(--fill-primary)" }}>
        {intentTitle}
      </div>

      {/* Command / Path always visible */}
      {data.action?.command && (
        <div className="mb-1">
          <pre
            className="text-xs leading-relaxed whitespace-pre-wrap break-all"
            style={{
              color: "var(--fill-primary)",
              fontFamily: "var(--font-mono, ui-monospace, monospace)",
            }}
          >
            {data.action.command}
          </pre>
          {data.action.cwd && (
            <div
              className="mt-0.5 text-[11px]"
              style={{ color: "var(--fill-tertiary)", fontFamily: "var(--font-mono, ui-monospace, monospace)" }}
            >
              {data.action.cwd}
            </div>
          )}
        </div>
      )}

      {data.action?.path && !data.action?.command && (
        <div className="mb-1">
          <span
            className="text-xs"
            style={{ color: "var(--fill-primary)", fontFamily: "var(--font-mono, ui-monospace, monospace)" }}
          >
            {data.action.path}
          </span>
        </div>
      )}

      {/* MCP tool call details */}
      {data.action?.action_type === "mcp_tool_call" && data.action?.server_id && (
        <div className="mb-2">
          <div className="flex items-center gap-2 text-xs" style={{ color: "var(--fill-secondary)" }}>
            <span
              className="inline-flex items-center rounded px-1.5 py-0.5 text-[11px] font-medium"
              style={{ background: "var(--bg-elevated)", color: "var(--fill-primary)" }}
            >
              {data.action.server_id}
            </span>
            <span style={{ color: "var(--fill-tertiary)" }}>/</span>
            <span style={{ fontFamily: "var(--font-mono, ui-monospace, monospace)", color: "var(--fill-primary)" }}>
              {data.action.tool_name}
            </span>
          </div>
          {data.action.arguments_summary && (
            <pre
              className="mt-1.5 max-h-32 overflow-auto rounded-md p-2 text-xs leading-relaxed"
              style={{
                background: "var(--bg-code, rgba(0,0,0,0.04))",
                color: "var(--fill-primary)",
                fontFamily: "var(--font-mono, ui-monospace, monospace)",
                whiteSpace: "pre-wrap",
                wordBreak: "break-all",
              }}
            >
              {data.action.arguments_summary}
            </pre>
          )}
        </div>
      )}

      {/* Content/diff expandable preview */}
      {previewContent && (
        <div className="mt-2 mb-2">
          <button
            className="flex items-center gap-1 text-[11px] font-medium"
            style={{ color: "var(--fill-tertiary)" }}
            onClick={() => setPreviewExpanded(!previewExpanded)}
          >
            {previewExpanded ? <CaretUp size={12} /> : <CaretDown size={12} />}
            {data.action?.diff ? t("approval_showDiff") : t("approval_showContent")}
          </button>
          {previewExpanded && (
            <pre
              className="mt-1.5 max-h-48 overflow-auto rounded-md p-2.5 text-xs leading-relaxed"
              style={{
                background: "var(--bg-code, rgba(0,0,0,0.04))",
                color: "var(--fill-primary)",
                fontFamily: "var(--font-mono, ui-monospace, monospace)",
              }}
            >
              {previewContent}
            </pre>
          )}
        </div>
      )}

      {/* Decision list (vertical) */}
      <div className="mt-3 flex flex-col gap-1.5">
        {data.decisions.map((d) => {
          const shortcut = shortcutForDecision(d.id);
          const isSubmitted = submitted === d.id;
          const isDisabled = submitted !== null && !isSubmitted;

          if (d.id === "approved_with_policy_amend") {
            if (!d.prefix || d.prefix.length === 0) return null;
            const prefix = d.prefix.join(" ");
            return (
              <button
                key={d.id}
                onClick={() => handleDecision(d.id)}
                disabled={!!submitted}
                className="flex items-center gap-2 rounded-md px-2.5 py-1.5 text-xs transition-opacity"
                style={{
                  background: isSubmitted ? "var(--color-blue-100, rgba(66, 153, 225, 0.15))" : "transparent",
                  color: isDisabled ? "var(--fill-quaternary)" : "var(--fill-secondary)",
                  opacity: isDisabled ? 0.4 : 1,
                  border: "1px solid var(--separator)",
                }}
              >
                {shortcut && (
                  <kbd className="inline-flex h-5 w-5 items-center justify-center rounded text-[10px] font-bold uppercase" style={{ background: "var(--bg-elevated)", color: "var(--fill-tertiary)", border: "1px solid var(--separator)" }}>
                    {shortcut}
                  </kbd>
                )}
                <span>{t("approval_rememberPrefix", { prefix })}</span>
              </button>
            );
          }

          return (
            <button
              key={d.id}
              onClick={() => handleDecision(d.id)}
              disabled={!!submitted}
              className="flex items-center gap-2 rounded-md px-2.5 py-1.5 text-xs transition-opacity"
              style={{
                background: isSubmitted ? getDecisionHighlight(d.id) : "transparent",
                color: isDisabled ? "var(--fill-quaternary)" : "var(--fill-secondary)",
                opacity: isDisabled ? 0.4 : 1,
                border: "1px solid var(--separator)",
              }}
            >
              {shortcut && (
                <kbd className="inline-flex h-5 w-5 items-center justify-center rounded text-[10px] font-bold uppercase" style={{ background: "var(--bg-elevated)", color: "var(--fill-tertiary)", border: "1px solid var(--separator)" }}>
                  {shortcut}
                </kbd>
              )}
              <span>{DECISION_LABEL_KEYS[d.id] ? t(DECISION_LABEL_KEYS[d.id]) : d.label}</span>
            </button>
          );
        })}

        {/* ApprovedAllForSession — no shortcut, deliberate click only */}
        {sessionId && !submitted && (
          <button
            onClick={handleApproveAllForSession}
            className="mt-1 flex items-center gap-2 rounded-md px-2.5 py-1.5 text-xs transition-opacity"
            style={{
              background: "transparent",
              color: "var(--fill-tertiary)",
              border: "1px dashed var(--separator)",
            }}
          >
            <span>{t("approval_approveAllSessionSkip")}</span>
          </button>
        )}
      </div>
    </div>
  );
}

function getDecisionHighlight(decision: string): string {
  switch (decision) {
    case "approved":
      return "var(--color-green-100, rgba(72, 187, 120, 0.15))";
    case "approved_for_session":
      return "var(--color-blue-100, rgba(66, 153, 225, 0.15))";
    case "denied":
      return "var(--color-red-100, rgba(245, 101, 101, 0.15))";
    case "abort":
      return "var(--color-red-100, rgba(245, 101, 101, 0.15))";
    default:
      return "var(--bg-elevated, rgba(0,0,0,0.06))";
  }
}
