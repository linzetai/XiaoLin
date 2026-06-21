import { useState, useMemo, useCallback, useEffect, useRef } from "react";
import { useTranslation } from "react-i18next";
import {
  Compass,
  Code,
  CaretDown,
  CaretUp,
  FileText,
  ArrowsClockwise,
  ChatText,
  ArrowSquareOut,
  Eraser,
} from "@phosphor-icons/react";
import Markdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { open } from "@tauri-apps/plugin-shell";
import { useChatMetaStore, useStreamStore } from "../../lib/stores";
import * as transport from "../../lib/transport";
import { ICON_SIZE } from "../../lib/ui-tokens";

const remarkPlugins = [remarkGfm];

const PREF_KEY = "xiaolin:plan-approval-preference";

export interface PlanApprovalMetadata {
  approval_pending?: boolean;
  plan_path?: string;
  plan_exists?: boolean;
}

export function isPlanExitResult(toolName: string, _result: string, metadata?: PlanApprovalMetadata | null): boolean {
  if (toolName !== "exit_plan_mode") return false;
  return metadata?.approval_pending === true;
}

type ActionKey = "implement" | "clear_implement" | "feedback" | "continue" | "open_editor";

export function PlanApprovalCard({
  result,
  metadata,
  onApprove,
}: {
  result: string;
  metadata?: PlanApprovalMetadata | null;
  onApprove?: (mode: "agent" | "plan") => void;
}) {
  const { t } = useTranslation("chat");
  const [expanded, setExpanded] = useState(true);
  const [loading, setLoading] = useState(false);
  const [planContent, setPlanContent] = useState<string | null>(null);
  const [approved, setApproved] = useState<string | null>(null);
  const [feedbackOpen, setFeedbackOpen] = useState(false);
  const [feedbackText, setFeedbackText] = useState("");
  const [rememberChoice, setRememberChoice] = useState(
    () => localStorage.getItem(PREF_KEY) !== null,
  );
  const [countdown, setCountdown] = useState<number | null>(null);
  const countdownRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const feedbackRef = useRef<HTMLTextAreaElement>(null);

  const activeChatId = useChatMetaStore((s) => s.activeChatId);
  const usage = useStreamStore((s) => s.usage[activeChatId]);

  const contextPct = useMemo(() => {
    if (!usage?.contextTokens || !usage?.contextWindow) return null;
    return Math.round((usage.contextTokens / usage.contextWindow) * 100);
  }, [usage?.contextTokens, usage?.contextWindow]);

  const planPath = useMemo(() => {
    if (metadata?.plan_path) return metadata.plan_path;
    const match = result.match(/Saved at:\s*(.+?)[\n\r]/);
    return match?.[1]?.trim() ?? null;
  }, [result, metadata]);

  const inlinePreview = useMemo(() => {
    const idx = result.indexOf("## Plan File");
    if (idx < 0) return null;
    const afterHeader = result.slice(idx);
    const lines = afterHeader.split("\n").slice(2);
    const content = lines.join("\n").replace(/\n\nThe user will review.*$/, "").replace(/\n\nYou can refer back.*$/, "").trim();
    return content || null;
  }, [result]);

  const isPending = metadata?.approval_pending ?? false;
  const isDisabled = approved !== null;

  // Auto-approve countdown logic
  useEffect(() => {
    if (!isPending || isDisabled) return;
    const savedPref = localStorage.getItem(PREF_KEY) as ActionKey | null;
    if (!savedPref || savedPref === "feedback" || savedPref === "open_editor") return;

    setCountdown(3);
    countdownRef.current = setInterval(() => {
      setCountdown((prev) => {
        if (prev === null || prev <= 1) {
          if (countdownRef.current) clearInterval(countdownRef.current);
          return 0;
        }
        return prev - 1;
      });
    }, 500);

    return () => {
      if (countdownRef.current) clearInterval(countdownRef.current);
    };
  }, [isPending, isDisabled]);

  const cancelCountdown = useCallback(() => {
    if (countdownRef.current) clearInterval(countdownRef.current);
    setCountdown(null);
  }, []);

  const handleExpand = useCallback(async () => {
    if (expanded) {
      setExpanded(false);
      return;
    }
    setExpanded(true);
    if (planContent) return;
    setLoading(true);
    try {
      const chatId = useChatMetaStore.getState().activeChatId;
      const resp = await transport.getPlanFile(chatId ?? undefined);
      setPlanContent(resp.content ?? inlinePreview ?? t("plan_empty"));
    } catch {
      setPlanContent(inlinePreview ?? t("plan_readFailed"));
    } finally {
      setLoading(false);
    }
  }, [expanded, planContent, inlinePreview, t]);

  const executeAction = useCallback(async (action: ActionKey) => {
    if (isDisabled) return;
    cancelCountdown();

    const sessionId = useChatMetaStore.getState().activeChatId;
    const setChatExecutionMode = useChatMetaStore.getState().setChatExecutionMode;
    const updateChatBackendId = useChatMetaStore.getState().updateChatBackendId;

    if (rememberChoice && action !== "open_editor") {
      localStorage.setItem(PREF_KEY, action);
    }

    switch (action) {
      case "implement": {
        setApproved(t("plan_startImplementation"));
        if (onApprove) {
          onApprove("agent");
        } else {
          await transport.approvePlan(sessionId, "agent");
          setChatExecutionMode(sessionId, "agent");
          window.dispatchEvent(new CustomEvent("xiaolin:plan-approved", {
            detail: { planPath: planPath ?? "" },
          }));
        }
        break;
      }
      case "clear_implement": {
        setApproved(t("plan_clearContext"));
        const resp = await transport.approvePlan(sessionId, "agent", { clearContext: true });
        if (resp.newSessionId) {
          updateChatBackendId(sessionId, resp.newSessionId);
          setChatExecutionMode(resp.newSessionId, "agent");
          window.dispatchEvent(new CustomEvent("xiaolin:plan-approved", {
            detail: { planPath: planPath ?? "", newSessionId: resp.newSessionId },
          }));
        }
        break;
      }
      case "feedback": {
        if (!feedbackText.trim()) {
          setFeedbackOpen(true);
          setTimeout(() => feedbackRef.current?.focus(), 50);
          return;
        }
        setApproved(t("plan_feedbackSent"));
        await transport.rejectPlan(sessionId, feedbackText.trim());
        setFeedbackText("");
        setFeedbackOpen(false);
        break;
      }
      case "continue": {
        setApproved(t("plan_continuePlanning"));
        if (onApprove) {
          onApprove("plan");
        } else {
          await transport.approvePlan(sessionId, "plan");
          setChatExecutionMode(sessionId, "plan");
        }
        break;
      }
      case "open_editor": {
        if (planPath) {
          await open(planPath);
        }
        break;
      }
    }
  }, [isDisabled, cancelCountdown, rememberChoice, onApprove, planPath, feedbackText, t]);

  const executeActionRef = useRef(executeAction);
  executeActionRef.current = executeAction;

  useEffect(() => {
    if (countdown === 0 && !isDisabled) {
      const savedPref = localStorage.getItem(PREF_KEY) as ActionKey | null;
      if (savedPref) executeActionRef.current(savedPref);
    }
  }, [countdown, isDisabled]);

  const handleFeedbackKeyDown = useCallback((e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      executeAction("feedback");
    } else if (e.key === "Escape") {
      setFeedbackOpen(false);
    }
  }, [executeAction]);

  const displayPath = planPath?.replace(/^\/home\/[^/]+\//, "~/") ?? "";

  return (
    <div
      className="overflow-hidden rounded-lg"
      style={{
        border: "0.5px solid var(--plan-tint-border)",
        borderLeft: "3px solid var(--plan-tint)",
        background: "var(--plan-tint-bg)",
      }}
    >
      {/* Header */}
      <div className="flex items-center gap-2 px-3 py-2">
        <Compass size={ICON_SIZE.md} style={{ color: "var(--plan-tint)" }} className="shrink-0" />
        <span className="text-[12px] font-semibold" style={{ color: "var(--plan-tint)" }}>
          {approved ? `✓ ${approved}` : isPending ? t("plan_pendingApproval") : t("plan_completed")}
        </span>
        {planPath && (
          <span
            className="min-w-0 truncate font-mono text-[10px]"
            style={{ color: "var(--fill-tertiary)" }}
            title={planPath}
          >
            {displayPath}
          </span>
        )}
      </div>

      {/* Collapsible markdown preview */}
      {inlinePreview && (
        <button
          onClick={handleExpand}
          className="flex w-full items-center gap-1.5 px-3 py-1.5 text-left text-[11px] font-medium transition-colors duration-100 hover:bg-[var(--plan-tint-subtle)]"
          style={{ color: "var(--fill-tertiary)", borderTop: "0.5px solid var(--separator)" }}
        >
          <FileText />
          <span>{expanded ? t("plan_collapse") : t("plan_viewContent")}</span>
          {expanded ? <CaretUp /> : <CaretDown />}
        </button>
      )}

      {expanded && (
        <div
          className="px-3 pb-3"
          style={{
            borderTop: "0.5px solid var(--separator)",
          }}
        >
          {loading ? (
            <div className="flex items-center gap-2 py-3 text-[11px]" style={{ color: "var(--fill-tertiary)" }}>
              <span
                className="inline-block h-3 w-3 rounded-full border-[1.5px]"
                style={{
                  borderColor: "var(--fill-tertiary) transparent transparent transparent",
                  animation: "spin 0.8s linear infinite",
                }}
              />
              {t("plan_loading")}
            </div>
          ) : (
            <div
              className="mt-2 max-h-[600px] overflow-y-auto rounded-md p-3 text-[12px] leading-[1.6]"
              style={{
                background: "var(--bg-primary)",
                border: "0.5px solid var(--separator)",
                color: "var(--fill-secondary)",
              }}
            >
              <Markdown remarkPlugins={remarkPlugins}>{planContent ?? inlinePreview ?? ""}</Markdown>
            </div>
          )}
        </div>
      )}

      {/* Action buttons */}
      {isPending && (
        <div
          className="flex flex-col gap-1.5 px-3 py-2"
          style={{ borderTop: "0.5px solid var(--separator)" }}
        >
          {/* Auto-approve countdown */}
          {countdown !== null && countdown > 0 && (
            <div className="flex items-center gap-2 pb-1 text-[10px]" style={{ color: "var(--fill-tertiary)" }}>
              <span>{t("plan_autoApproving", { seconds: Math.ceil(countdown * 0.5) })}...</span>
              <button
                onClick={cancelCountdown}
                className="rounded px-1.5 py-0.5 text-[10px] font-medium"
                style={{ color: "var(--plan-tint)", background: "var(--plan-tint-bg)" }}
              >
                {t("plan_cancel")}
              </button>
            </div>
          )}

          {/* Primary: Start implementation */}
          <button
            onClick={() => executeAction("implement")}
            disabled={isDisabled}
            className="flex items-center gap-1.5 rounded-md px-3 py-1.5 text-[11px] font-medium transition-all duration-150 hover:scale-[1.02] active:scale-95 disabled:opacity-40 disabled:pointer-events-none"
            style={{
              background: "var(--green, #48BB78)",
              color: "#fff",
            }}
          >
            <Code size={14} />
            {t("plan_startImplementation")}
          </button>

          {/* Secondary: Clear context + implement */}
          <button
            onClick={() => executeAction("clear_implement")}
            disabled={isDisabled}
            className="flex items-center gap-1.5 rounded-md px-3 py-1.5 text-[11px] font-medium transition-all duration-150 hover:scale-[1.02] active:scale-95 disabled:opacity-40 disabled:pointer-events-none"
            style={{
              background: "var(--bg-secondary)",
              color: "var(--fill-primary)",
              border: "0.5px solid var(--separator)",
            }}
          >
            <Eraser size={14} />
            {t("plan_clearContext")}
            {contextPct !== null && (
              <span className="ml-1 text-[10px]" style={{ color: "var(--fill-tertiary)" }}>
                (ctx: {contextPct}%)
              </span>
            )}
          </button>

          {/* Secondary: Give feedback */}
          <button
            onClick={() => {
              if (feedbackOpen && feedbackText.trim()) {
                executeAction("feedback");
              } else {
                setFeedbackOpen(!feedbackOpen);
                if (!feedbackOpen) setTimeout(() => feedbackRef.current?.focus(), 50);
              }
            }}
            disabled={isDisabled}
            className="flex items-center gap-1.5 rounded-md px-3 py-1.5 text-[11px] font-medium transition-all duration-150 hover:scale-[1.02] active:scale-95 disabled:opacity-40 disabled:pointer-events-none"
            style={{
              background: "var(--bg-secondary)",
              color: "var(--fill-primary)",
              border: "0.5px solid var(--separator)",
            }}
          >
            <ChatText size={14} />
            {feedbackOpen && feedbackText.trim() ? t("plan_sendFeedback") : t("plan_giveFeedback")}
          </button>

          {/* Feedback textarea */}
          {feedbackOpen && (
            <div className="ml-5">
              <textarea
                ref={feedbackRef}
                value={feedbackText}
                onChange={(e) => setFeedbackText(e.target.value)}
                onKeyDown={handleFeedbackKeyDown}
                placeholder={t("plan_feedbackPlaceholder")}
                disabled={isDisabled}
                className="w-full resize-none rounded-md p-2 text-[11px] leading-[1.5] disabled:opacity-40"
                style={{
                  background: "var(--bg-primary)",
                  border: "0.5px solid var(--separator)",
                  color: "var(--fill-primary)",
                  minHeight: "60px",
                  maxHeight: "120px",
                  outline: "none",
                }}
                onFocus={(e) => { e.currentTarget.style.boxShadow = "0 0 0 2px var(--plan-tint)"; e.currentTarget.style.borderColor = "var(--plan-tint)"; }}
                onBlur={(e) => { e.currentTarget.style.boxShadow = "none"; e.currentTarget.style.borderColor = "var(--separator)"; }}
                rows={3}
              />
              <div className="mt-0.5 text-[9px]" style={{ color: "var(--fill-quaternary)" }}>
                Enter → {t("plan_send")} · Shift+Enter → {t("plan_newline")} · Esc → {t("plan_closeFeedback")}
              </div>
            </div>
          )}

          {/* Ghost: Continue planning */}
          <button
            onClick={() => executeAction("continue")}
            disabled={isDisabled}
            className="flex items-center gap-1.5 rounded-md px-3 py-1.5 text-[11px] font-medium transition-all duration-150 hover:bg-[var(--plan-tint-bg)] active:scale-95 disabled:opacity-40 disabled:pointer-events-none"
            style={{ color: "var(--fill-tertiary)" }}
          >
            <ArrowsClockwise size={14} />
            {t("plan_continuePlanning")}
          </button>

          {/* Ghost: Open in editor */}
          {planPath && (
            <button
              onClick={() => executeAction("open_editor")}
              disabled={isDisabled}
              className="flex items-center gap-1.5 rounded-md px-3 py-1.5 text-[11px] font-medium transition-all duration-150 hover:bg-[var(--plan-tint-bg)] active:scale-95 disabled:opacity-40 disabled:pointer-events-none"
              style={{ color: "var(--fill-tertiary)" }}
            >
              <ArrowSquareOut size={14} />
              {t("plan_openInEditor")}
            </button>
          )}

          {/* Remember choice */}
          <label className="mt-1 flex items-center gap-1.5 text-[10px] cursor-pointer" style={{ color: "var(--fill-quaternary)" }}>
            <input
              type="checkbox"
              checked={rememberChoice}
              onChange={(e) => {
                setRememberChoice(e.target.checked);
                if (!e.target.checked) {
                  localStorage.removeItem(PREF_KEY);
                }
              }}
              className="rounded"
              style={{ accentColor: "var(--plan-tint)" }}
            />
            {t("plan_rememberChoice")}
          </label>
        </div>
      )}
    </div>
  );
}
