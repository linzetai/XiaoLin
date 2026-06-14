import { useState, useCallback, useEffect, useRef } from "react";
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

function getIntentTitle(action?: ApprovalData["action"]): string {
  switch (action?.action_type) {
    case "shell_command":
      return "允许执行此命令？";
    case "file_write":
      return "允许写入此文件？";
    case "apply_patch":
      return "允许修改此文件？";
    case "network_access":
      return "允许网络访问？";
    default:
      return "需要你的许可";
  }
}

function shortcutForDecision(id: string): string | null {
  for (const [key, val] of Object.entries(SHORTCUT_MAP)) {
    if (val === id) return key;
  }
  return null;
}

export function ApprovalCard({ data, onDecision, sessionId }: ApprovalCardProps) {
  const [submitted, setSubmitted] = useState<string | null>(null);
  const submittedRef = useRef(false);
  const setSessionPreset = usePermissionStore((s) => s.setSessionPreset);

  const borderColor = RISK_COLORS[data.riskLevel] ?? RISK_COLORS.medium;
  const intentTitle = getIntentTitle(data.action);

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

      {/* Content/diff expandable preview */}
      {previewContent && (
        <div className="mt-2 mb-2">
          <button
            className="flex items-center gap-1 text-[11px] font-medium"
            style={{ color: "var(--fill-tertiary)" }}
            onClick={() => setPreviewExpanded(!previewExpanded)}
          >
            {previewExpanded ? <CaretUp size={12} /> : <CaretDown size={12} />}
            {data.action?.diff ? "显示变更" : "显示内容"}
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
                <span>记住「{prefix}」前缀，以后自动允许</span>
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
              <span>{d.label}</span>
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
            <span>本次全部允许（跳过后续所有审批）</span>
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
