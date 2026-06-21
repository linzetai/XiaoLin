import { useState, useEffect, useCallback, useRef, useMemo } from "react";
import { useTranslation } from "react-i18next";
import { X, FileText, ArrowsClockwise, Circle, CircleNotch, CheckCircle, Compass } from "@phosphor-icons/react";
import Markdown from "react-markdown";
import remarkGfm from "remark-gfm";
import * as transport from "../../lib/transport";
import { onWsEvent } from "../../lib/transport";
import { ICON_SIZE } from "../../lib/ui-tokens";
import { useChatMetaStore } from "../../lib/stores/chat-meta-store";
import { useWorkspaceTabs } from "../shell/workspace-tabs";
import type { PlanStep, PlanUpdateData } from "../../lib/stores/types";

const remarkPlugins = [remarkGfm];

interface PlanPanelProps {
  sessionId: string;
  planFilePath?: string;
  planFileExists?: boolean;
  onClose: () => void;
}

export function PlanPanel({ sessionId, planFilePath, planFileExists, onClose }: PlanPanelProps) {
  const { t } = useTranslation("chat");
  const [content, setContent] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  // Streaming state
  const [isStreaming, setIsStreaming] = useState(false);
  const [stableContent, setStableContent] = useState("");
  const bufferRef = useRef("");

  // Structured plan steps
  const [planSteps, setPlanSteps] = useState<PlanStep[]>([]);
  const [planExplanation, setPlanExplanation] = useState<string | undefined>();

  // Auto-scroll state
  const scrollContainerRef = useRef<HTMLDivElement>(null);
  const userScrolledRef = useRef(false);
  const SCROLL_THRESHOLD = 40;

  const fetchContent = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const resp = await transport.getPlanFile(sessionId);
      setContent(resp.content);
    } catch (err) {
      setError(err instanceof Error ? err.message : t("loadFailed"));
    } finally {
      setLoading(false);
    }
  }, [sessionId, t]);

  useEffect(() => {
    fetchContent();
  }, [fetchContent]);

  // Handle plan_file_update: finalize streaming mode
  useEffect(() => {
    const unsub = onWsEvent("plan_file_update", (msg: unknown) => {
      const data = (msg as { data?: { sessionId?: string; session_id?: string; content?: string } })?.data;
      const evtSessionId = data?.sessionId ?? data?.session_id;
      if (evtSessionId === sessionId) {
        setIsStreaming(false);
        bufferRef.current = "";
        if (typeof data?.content === "string") {
          setContent(data.content);
          setStableContent("");
          setLoading(false);
          setError(null);
        } else {
          fetchContent();
        }
      }
    });
    return unsub;
  }, [sessionId, fetchContent]);

  // Handle plan_delta: streaming content
  useEffect(() => {
    const unsub = onWsEvent("plan_delta", (msg: unknown) => {
      const data = (msg as { data?: { delta?: string } })?.data;
      if (!data?.delta) return;

      setIsStreaming(true);
      setLoading(false);
      setError(null);

      bufferRef.current += data.delta;

      // Line-commit strategy: commit up to the last newline
      const lastNewline = bufferRef.current.lastIndexOf("\n");
      if (lastNewline >= 0) {
        const toCommit = bufferRef.current.slice(0, lastNewline + 1);
        bufferRef.current = bufferRef.current.slice(lastNewline + 1);
        setStableContent((prev) => prev + toCommit);
      }
    });
    return unsub;
  }, []);

  // Handle plan_update: structured step updates
  useEffect(() => {
    const unsub = onWsEvent("plan_update", (msg: unknown) => {
      const data = (msg as { data?: PlanUpdateData })?.data;
      if (!data?.steps) return;
      setPlanSteps(data.steps);
      setPlanExplanation(data.explanation ?? undefined);
    });
    return unsub;
  }, []);

  // Auto-scroll on content update
  useEffect(() => {
    if (!isStreaming || userScrolledRef.current) return;
    const el = scrollContainerRef.current;
    if (el) {
      el.scrollTop = el.scrollHeight;
    }
  }, [stableContent, isStreaming]);

  // Detect user scroll
  const handleScroll = useCallback(() => {
    const el = scrollContainerRef.current;
    if (!el) return;
    const atBottom = el.scrollTop + el.clientHeight >= el.scrollHeight - SCROLL_THRESHOLD;
    userScrolledRef.current = !atBottom;
  }, []);

  // Reset user scroll on stream start
  useEffect(() => {
    if (isStreaming) {
      userScrolledRef.current = false;
    }
  }, [isStreaming]);

  const displayPath = planFilePath?.replace(/^\/home\/[^/]+\//, "~/") ?? "";

  // Determine what to render
  const renderContent = isStreaming
    ? stableContent + bufferRef.current
    : content;

  const filteredContent = useMemo(() => {
    if (!renderContent || planSteps.length === 0) return renderContent;
    const lines = renderContent.split("\n");
    const kept = lines.filter(line => !/^\s*-\s*\[[ x~]\]\s/i.test(line));
    const result = kept.join("\n").trim();
    return result || null;
  }, [renderContent, planSteps.length]);

  return (
    <div
      className="flex h-full flex-col"
      style={{
        background: "var(--bg-secondary)",
      }}
    >
      <div
        className="flex shrink-0 items-center gap-2 px-3 py-2"
        style={{
          borderBottom: "0.5px solid var(--separator)",
          background: "var(--plan-tint-bg)",
        }}
      >
        <FileText size={ICON_SIZE.md} style={{ color: "var(--plan-tint)" }} />
        <span className="flex-1 text-[12px] font-semibold" style={{ color: "var(--plan-tint)" }}>
          {t("plan_file")}
        </span>
        {isStreaming && (
          <span
            className="text-[10px] font-medium"
            style={{ color: "var(--plan-tint)", opacity: 0.7 }}
          >
            streaming…
          </span>
        )}
        <button
          onClick={fetchContent}
          className="rounded p-1 transition-colors hover:bg-[color-mix(in_srgb,var(--fill-tertiary)_10%,transparent)]"
          title={t("plan_refresh")}
        >
          <ArrowsClockwise style={{ color: "var(--fill-tertiary)" }} />
        </button>
        <button
          onClick={onClose}
          className="rounded p-1 transition-colors hover:bg-[color-mix(in_srgb,var(--fill-tertiary)_10%,transparent)]"
          title={t("close", { ns: "common" })}
        >
          <X style={{ color: "var(--fill-tertiary)" }} />
        </button>
      </div>

      {displayPath && (
        <div
          className="shrink-0 px-3 py-1.5 font-mono text-[10px]"
          style={{
            color: "var(--fill-tertiary)",
            borderBottom: "0.5px solid var(--separator)",
          }}
          title={planFilePath}
        >
          {displayPath}
          {planFileExists === false && (
            <span style={{ opacity: 0.6 }}> {t("plan_notCreated")}</span>
          )}
        </div>
      )}

      <div
        ref={scrollContainerRef}
        onScroll={handleScroll}
        className="min-h-0 flex-1 overflow-y-auto px-3 py-3"
      >
        {/* Structured plan steps checklist */}
        {planSteps.length > 0 && (
          <PlanChecklist steps={planSteps} explanation={planExplanation} />
        )}

        {loading && !isStreaming && (
          <div className="flex items-center gap-2 py-6 text-[11px]" style={{ color: "var(--fill-tertiary)" }}>
            <span
              className="inline-block h-3 w-3 rounded-full border-[1.5px]"
              style={{
                borderColor: "var(--fill-tertiary) transparent transparent transparent",
                animation: "spin 0.8s linear infinite",
              }}
            />
            {t("loading", { ns: "common" })}
          </div>
        )}
        {error && (
          <div className="py-4 text-[11px]" style={{ color: "var(--red, #E53E3E)" }}>
            {error}
          </div>
        )}
        {filteredContent && (
          <div
            className="plan-panel-content text-[12px] leading-[1.6]"
            style={{ color: "var(--fill-secondary)" }}
          >
            <Markdown remarkPlugins={remarkPlugins}>{filteredContent}</Markdown>
            {isStreaming && <StreamingCursor />}
          </div>
        )}
        {!loading && !isStreaming && !error && !content && (
          <div className="flex flex-col items-center gap-3 py-8">
            <Compass size={32} weight="light" style={{ color: "var(--plan-tint)", opacity: 0.5 }} />
            <div className="text-center text-[12px]" style={{ color: "var(--fill-tertiary)" }}>
              {t("plan_notCreatedYet")}
            </div>
            <div className="text-center text-[11px]" style={{ color: "var(--fill-quaternary)" }}>
              {t("plan_emptyHint")}
            </div>
          </div>
        )}
      </div>
    </div>
  );
}

function StreamingCursor() {
  return (
    <span
      className="streaming-cursor"
      style={{
        display: "inline-block",
        width: "2px",
        height: "1em",
        backgroundColor: "var(--plan-tint)",
        marginLeft: "1px",
        verticalAlign: "text-bottom",
        animation: "plan-cursor-blink 0.8s step-end infinite",
      }}
    />
  );
}

function PlanChecklist({ steps, explanation }: { steps: PlanStep[]; explanation?: string }) {
  const completed = steps.filter((s) => s.status === "completed").length;
  const total = steps.length;
  const progress = total > 0 ? Math.round((completed / total) * 100) : 0;

  return (
    <div
      className="mb-3 rounded-md border"
      style={{
        borderColor: "var(--separator)",
        background: "var(--bg-primary)",
      }}
    >
      {/* Header with progress */}
      <div
        className="flex items-center gap-2 px-3 py-2"
        style={{ borderBottom: "0.5px solid var(--separator)" }}
      >
        <span className="text-[11px] font-semibold" style={{ color: "var(--fill-secondary)" }}>
          进度
        </span>
        <div
          className="h-1.5 flex-1 rounded-full"
          style={{ background: "var(--fill-quaternary, #e2e8f0)" }}
        >
          <div
            className="h-full rounded-full transition-all duration-300"
            style={{
              width: `${progress}%`,
              background: "var(--plan-tint)",
            }}
          />
        </div>
        <span className="text-[10px] font-medium" style={{ color: "var(--fill-tertiary)" }}>
          {completed}/{total}
        </span>
      </div>

      {/* Steps list */}
      <ul className="list-none px-3 py-2">
        {steps.map((step, idx) => (
          <li key={idx} className="flex items-start gap-2 py-1">
            <StepIcon status={step.status} />
            <span
              className="text-[12px] leading-[1.5]"
              style={{
                color:
                  step.status === "completed"
                    ? "var(--fill-tertiary)"
                    : step.status === "in_progress"
                      ? "var(--plan-tint)"
                      : "var(--fill-secondary)",
                textDecoration: step.status === "completed" ? "line-through" : "none",
                fontWeight: step.status === "in_progress" ? 500 : 400,
              }}
            >
              {step.step}
            </span>
          </li>
        ))}
      </ul>

      {/* Explanation */}
      {explanation && (
        <div
          className="px-3 pb-2 text-[10px]"
          style={{ color: "var(--fill-tertiary)" }}
        >
          {explanation}
        </div>
      )}
    </div>
  );
}

function StepIcon({ status }: { status: PlanStep["status"] }) {
  if (status === "completed") {
    return <CheckCircle size={14} weight="fill" style={{ color: "var(--green, #38A169)", flexShrink: 0, marginTop: 2 }} />;
  }
  if (status === "in_progress") {
    return (
      <CircleNotch
        size={14}
        weight="bold"
        style={{
          color: "var(--plan-tint)",
          flexShrink: 0,
          marginTop: 2,
          animation: "spin 1s linear infinite",
        }}
      />
    );
  }
  return <Circle size={14} weight="regular" style={{ color: "var(--fill-tertiary)", flexShrink: 0, marginTop: 2 }} />;
}

/**
 * Workspace tab adapter — reads plan state from the chat-meta store
 * so it can be registered as a zero-prop ComponentType.
 */
export function PlanTabContent() {
  const activeChatId = useChatMetaStore((s) => s.activeChatId);
  const chat = useChatMetaStore((s) => s.chats[s.activeChatId]);
  const togglePanel = useWorkspaceTabs((s) => s.togglePanel);
  const setPlanClosedByUser = useWorkspaceTabs((s) => s.setPlanClosedByUser);

  const handleClose = useCallback(() => {
    setPlanClosedByUser(true);
    togglePanel();
  }, [setPlanClosedByUser, togglePanel]);

  if (!activeChatId || !chat) return null;

  return (
    <PlanPanel
      sessionId={activeChatId}
      planFilePath={chat.planFilePath}
      planFileExists={chat.planFileExists}
      onClose={handleClose}
    />
  );
}
