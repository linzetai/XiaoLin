import { useState, useEffect, useCallback } from "react";
import { X, FileText, RefreshCw } from "lucide-react";
import Markdown from "react-markdown";
import remarkGfm from "remark-gfm";
import * as transport from "../../lib/transport";
import { onWsEvent } from "../../lib/transport";
import { ICON } from "../../lib/ui-tokens";

const remarkPlugins = [remarkGfm];

interface PlanPanelProps {
  sessionId: string;
  planFilePath?: string;
  planFileExists?: boolean;
  onClose: () => void;
}

export function PlanPanel({ sessionId, planFilePath, planFileExists, onClose }: PlanPanelProps) {
  const [content, setContent] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const fetchContent = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const resp = await transport.getPlanFile(sessionId);
      setContent(resp.content);
    } catch (err) {
      setError(err instanceof Error ? err.message : "加载失败");
    } finally {
      setLoading(false);
    }
  }, [sessionId]);

  useEffect(() => {
    fetchContent();
  }, [fetchContent]);

  useEffect(() => {
    const unsub = onWsEvent("plan_file_update", (msg: unknown) => {
      const data = (msg as { data?: { sessionId?: string } })?.data;
      if (data?.sessionId === sessionId) {
        fetchContent();
      }
    });
    return unsub;
  }, [sessionId, fetchContent]);

  const displayPath = planFilePath?.replace(/^\/home\/[^/]+\//, "~/") ?? "";

  return (
    <div
      className="flex h-full flex-col"
      style={{
        background: "var(--bg-secondary)",
        borderLeft: "0.5px solid var(--separator)",
      }}
    >
      <div
        className="flex shrink-0 items-center gap-2 px-3 py-2"
        style={{
          borderBottom: "0.5px solid var(--separator)",
          background: "color-mix(in srgb, var(--tint, #4299E1) 4%, transparent)",
        }}
      >
        <FileText {...ICON.md} style={{ color: "var(--tint, #4299E1)" }} />
        <span className="flex-1 text-[12px] font-semibold" style={{ color: "var(--tint, #4299E1)" }}>
          计划文件
        </span>
        <button
          onClick={fetchContent}
          className="rounded p-1 transition-colors hover:bg-[color-mix(in_srgb,var(--fill-tertiary)_10%,transparent)]"
          title="刷新"
        >
          <RefreshCw {...ICON.sm} style={{ color: "var(--fill-tertiary)" }} />
        </button>
        <button
          onClick={onClose}
          className="rounded p-1 transition-colors hover:bg-[color-mix(in_srgb,var(--fill-tertiary)_10%,transparent)]"
          title="关闭"
        >
          <X {...ICON.sm} style={{ color: "var(--fill-tertiary)" }} />
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
            <span style={{ opacity: 0.6 }}> (未创建)</span>
          )}
        </div>
      )}

      <div className="min-h-0 flex-1 overflow-y-auto px-3 py-3">
        {loading && (
          <div className="flex items-center gap-2 py-6 text-[11px]" style={{ color: "var(--fill-tertiary)" }}>
            <span
              className="inline-block h-3 w-3 rounded-full border-[1.5px]"
              style={{
                borderColor: "var(--fill-tertiary) transparent transparent transparent",
                animation: "spin 0.8s linear infinite",
              }}
            />
            加载中...
          </div>
        )}
        {error && (
          <div className="py-4 text-[11px]" style={{ color: "var(--red, #E53E3E)" }}>
            {error}
          </div>
        )}
        {!loading && !error && content && (
          <div
            className="text-[12px] leading-[1.6]"
            style={{ color: "var(--fill-secondary)" }}
          >
            <Markdown remarkPlugins={remarkPlugins}>{content}</Markdown>
          </div>
        )}
        {!loading && !error && !content && (
          <div className="py-4 text-center text-[11px]" style={{ color: "var(--fill-tertiary)" }}>
            计划文件尚未创建
          </div>
        )}
      </div>
    </div>
  );
}
