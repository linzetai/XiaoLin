import { useState, useMemo, useCallback } from "react";
import { Compass, Code2, ChevronDown, ChevronUp, FileText } from "lucide-react";
import Markdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { useAgentStore } from "../../lib/agent-store";
import * as transport from "../../lib/transport";

const remarkPlugins = [remarkGfm];

export function isPlanExitResult(toolName: string, result: string): boolean {
  if (toolName !== "exit_plan_mode") return false;
  return result.includes("agent mode") && result.includes("Plan File");
}

export function PlanApprovalCard({
  result,
  onImplement,
}: {
  result: string;
  onImplement?: () => void;
}) {
  const [expanded, setExpanded] = useState(false);
  const [loading, setLoading] = useState(false);
  const [planContent, setPlanContent] = useState<string | null>(null);

  const planPath = useMemo(() => {
    const match = result.match(/Saved at:\s*(.+?)[\n\r]/);
    return match?.[1]?.trim() ?? null;
  }, [result]);

  const inlinePreview = useMemo(() => {
    const idx = result.indexOf("## Plan File");
    if (idx < 0) return null;
    const afterHeader = result.slice(idx);
    const lines = afterHeader.split("\n").slice(2);
    const content = lines.join("\n").replace(/\n\nYou can refer back.*$/, "").trim();
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
      const state = useAgentStore.getState();
      const agentId = state.activeAgentId;
      const ac = state.agentChats[agentId];
      const chatId = ac?.activeChatId;
      const resp = await transport.getPlanFileIpc(chatId ?? undefined);
      setPlanContent(resp.content ?? inlinePreview ?? "(计划文件为空)");
    } catch {
      setPlanContent(inlinePreview ?? "(无法读取计划文件)");
    } finally {
      setLoading(false);
    }
  }, [expanded, planContent, inlinePreview]);

  const displayPath = planPath?.replace(/^\/home\/[^/]+\//, "~/") ?? "";

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
        <Compass size={16} strokeWidth={2} style={{ color: "var(--tint, #4299E1)" }} className="shrink-0" />
        <span className="text-[12px] font-semibold" style={{ color: "var(--tint, #4299E1)" }}>
          计划已完成
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
          <FileText size={12} strokeWidth={1.5} />
          <span>{expanded ? "收起计划" : "查看计划内容"}</span>
          {expanded ? <ChevronUp size={12} strokeWidth={2} /> : <ChevronDown size={12} strokeWidth={2} />}
        </button>
      )}

      {expanded && (
        <div
          className="px-3 pb-3"
          style={{
            borderTop: "0.5px solid var(--separator)",
            animation: "fade-in var(--duration-instant) var(--ease-out)",
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
              加载计划内容...
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

      {onImplement && (
        <div
          className="flex items-center gap-2 px-3 py-2"
          style={{ borderTop: "0.5px solid var(--separator)" }}
        >
          <button
            onClick={onImplement}
            className="flex items-center gap-1.5 rounded-md px-3 py-1.5 text-[11px] font-medium transition-all duration-150 hover:scale-[1.02] active:scale-95"
            style={{
              background: "var(--green, #48BB78)",
              color: "#fff",
            }}
          >
            <Code2 size={13} strokeWidth={2} />
            开始实现
          </button>
          <span className="text-[10px]" style={{ color: "var(--fill-quaternary)" }}>
            切换到 Agent 模式并执行计划
          </span>
        </div>
      )}
    </div>
  );
}
