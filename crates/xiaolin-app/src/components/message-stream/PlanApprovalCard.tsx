import { useState, useMemo, useCallback } from "react";
import { useTranslation } from "react-i18next";
import { Compass, Code, CaretDown, CaretUp, FileText, ArrowsClockwise } from "@phosphor-icons/react";
import Markdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { useChatMetaStore } from "../../lib/stores";
import * as transport from "../../lib/transport";
import { ICON_SIZE } from "../../lib/ui-tokens";

const remarkPlugins = [remarkGfm];

export interface PlanApprovalMetadata {
  approval_pending?: boolean;
  plan_path?: string;
  plan_exists?: boolean;
}

export function isPlanExitResult(toolName: string, result: string, metadata?: PlanApprovalMetadata | null): boolean {
  if (toolName !== "exit_plan_mode") return false;
  if (metadata?.approval_pending) return true;
  return result.includes("approval") || result.includes("agent mode");
}

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
  const [expanded, setExpanded] = useState(false);
  const [loading, setLoading] = useState(false);
  const [planContent, setPlanContent] = useState<string | null>(null);
  const [approving, setApproving] = useState(false);

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

  const handleApprove = useCallback(async (mode: "agent" | "plan") => {
    if (approving) return;
    setApproving(true);
    try {
      if (onApprove) {
        onApprove(mode);
      } else {
        const { activeChatId: sessionId, setChatExecutionMode } = useChatMetaStore.getState();
        await transport.approvePlan(sessionId, mode);
        setChatExecutionMode(sessionId, mode);
      }
    } finally {
      setApproving(false);
    }
  }, [approving, onApprove]);

  const displayPath = planPath?.replace(/^\/home\/[^/]+\//, "~/") ?? "";
  const isPending = metadata?.approval_pending ?? false;

  return (
    <div
      className="overflow-hidden rounded-lg"
      style={{
        border: "0.5px solid color-mix(in srgb, var(--tint, #4299E1) 30%, transparent)",
        borderLeft: "3px solid var(--tint, #4299E1)",
        background: "color-mix(in srgb, var(--tint, #4299E1) 4%, transparent)",
      }}
    >
      <div className="flex items-center gap-2 px-3 py-2">
        <Compass size={ICON_SIZE.md} style={{ color: "var(--tint, #4299E1)" }} className="shrink-0" />
        <span className="text-[12px] font-semibold" style={{ color: "var(--tint, #4299E1)" }}>
          {isPending ? t("plan_pendingApproval") : t("plan_completed")}
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

      {inlinePreview && (
        <button
          onClick={handleExpand}
          className="flex w-full items-center gap-1.5 px-3 py-1.5 text-left text-[11px] font-medium transition-colors duration-100 hover:bg-[color-mix(in_srgb,var(--tint,#4299E1)_8%,transparent)]"
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
              className="mt-2 max-h-[400px] overflow-y-auto rounded-md p-3 text-[12px] leading-[1.6]"
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

      {isPending && (
        <div
          className="flex items-center gap-2 px-3 py-2"
          style={{ borderTop: "0.5px solid var(--separator)" }}
        >
          <button
            onClick={() => handleApprove("agent")}
            disabled={approving}
            className="flex items-center gap-1.5 rounded-md px-3 py-1.5 text-[11px] font-medium transition-all duration-150 hover:scale-[1.02] active:scale-95 disabled:opacity-50"
            style={{
              background: "var(--green, #48BB78)",
              color: "#fff",
            }}
          >
            <Code />
            {t("plan_startImplementation")}
          </button>
          <button
            onClick={() => handleApprove("plan")}
            disabled={approving}
            className="flex items-center gap-1.5 rounded-md px-3 py-1.5 text-[11px] font-medium transition-all duration-150 hover:scale-[1.02] active:scale-95 disabled:opacity-50"
            style={{
              background: "color-mix(in srgb, var(--tint, #4299E1) 15%, transparent)",
              color: "var(--tint, #4299E1)",
            }}
          >
            <ArrowsClockwise />
            {t("plan_continuePlanning")}
          </button>
          <span className="text-[10px]" style={{ color: "var(--fill-quaternary)" }}>
            {t("plan_chooseNext")}
          </span>
        </div>
      )}
    </div>
  );
}
