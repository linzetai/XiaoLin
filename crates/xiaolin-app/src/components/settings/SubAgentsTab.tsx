import { useState, useEffect, useCallback } from "react";
import { Bot, RefreshCw } from "lucide-react";
import { ICON, BTN_ICON } from "../../lib/ui-tokens";
import * as wsClient from "../../lib/ws-client";

interface SubAgentDef {
  id: string;
  name?: string;
  description?: string;
  background: boolean;
  concurrency_safe: boolean;
  tools?: {
    allowed?: string[];
    denied?: string[];
  };
  source?: string;
}

export function SubAgentsTab() {
  const [defs, setDefs] = useState<SubAgentDef[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const fetchDefs = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const resp = (await wsClient.send("sub_agents.list")) as {
        data?: { agents?: SubAgentDef[] };
      };
      setDefs(resp?.data?.agents ?? []);
    } catch (e) {
      setError(e instanceof Error ? e.message : "加载失败");
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    fetchDefs();
  }, [fetchDefs]);

  return (
    <div className="space-y-5">
      <div className="flex items-center justify-between">
        <div>
          <h3 className="text-[13px] font-semibold" style={{ color: "var(--fill-primary)" }}>
            Sub-Agent 定义
          </h3>
          <p className="mt-0.5 text-[12px]" style={{ color: "var(--fill-tertiary)" }}>
            主 Agent 可通过工具调用编排这些 Sub-Agent 来处理不同类型的子任务
          </p>
        </div>
        <button
          onClick={fetchDefs}
          disabled={loading}
          className={`${BTN_ICON.sm} cursor-pointer`}
          style={{ color: "var(--fill-tertiary)" }}
          title="刷新"
        >
          <RefreshCw {...ICON.sm} className={loading ? "animate-spin" : ""} />
        </button>
      </div>

      {error && (
        <div
          className="rounded-[var(--radius-xs)] px-3 py-2 text-[12px]"
          style={{ background: "var(--red-bg)", color: "var(--red)" }}
        >
          {error}
        </div>
      )}

      <div className="space-y-2">
        {defs.map((def) => (
          <div
            key={def.id}
            className="rounded-[var(--radius-sm)] p-3"
            style={{
              background: "var(--bg-base)",
              border: "0.5px solid var(--separator-opaque)",
            }}
          >
            <div className="flex items-center gap-2">
              <Bot {...ICON.md} style={{ color: "var(--tint)", flexShrink: 0 }} />
              <div className="min-w-0 flex-1">
                <div className="flex items-center gap-2">
                  <span className="text-[13px] font-semibold" style={{ color: "var(--fill-primary)" }}>
                    {def.name || def.id}
                  </span>
                  <span
                    className="rounded-full px-1.5 py-0.5 text-[10px] font-medium"
                    style={{
                      background: "var(--bg-tertiary)",
                      color: "var(--fill-quaternary)",
                    }}
                  >
                    {def.source || "builtin"}
                  </span>
                </div>
                {def.description && (
                  <p className="mt-0.5 text-[12px]" style={{ color: "var(--fill-tertiary)" }}>
                    {def.description}
                  </p>
                )}
              </div>
            </div>

            <div className="mt-2 flex flex-wrap gap-1.5">
              <span
                className="rounded-full px-2 py-0.5 text-[10px] font-medium"
                style={{
                  background: def.background ? "var(--orange-bg)" : "var(--green-bg)",
                  color: def.background ? "var(--orange)" : "var(--green)",
                }}
              >
                {def.background ? "异步" : "同步"}
              </span>
              {def.concurrency_safe && (
                <span
                  className="rounded-full px-2 py-0.5 text-[10px] font-medium"
                  style={{ background: "var(--blue-bg)", color: "var(--blue)" }}
                >
                  并发安全
                </span>
              )}
              {def.tools?.allowed && def.tools.allowed.length > 0 && (
                <span
                  className="rounded-full px-2 py-0.5 text-[10px] font-medium"
                  style={{ background: "var(--bg-tertiary)", color: "var(--fill-quaternary)" }}
                >
                  {def.tools.allowed.length} 工具
                </span>
              )}
            </div>
          </div>
        ))}

        {!loading && defs.length === 0 && !error && (
          <div className="py-6 text-center text-[12px]" style={{ color: "var(--fill-quaternary)" }}>
            暂无 Sub-Agent 定义
          </div>
        )}
      </div>

      <div
        className="rounded-[var(--radius-sm)] p-3"
        style={{
          background: "var(--bg-hover)",
          border: "0.5px solid var(--separator)",
        }}
      >
        <p className="text-[12px] leading-5" style={{ color: "var(--fill-tertiary)" }}>
          <strong>自定义 Sub-Agent</strong>：在项目根目录创建 <code className="font-mono">.xiaolin/agents/</code> 目录，
          放入 Markdown 文件（YAML frontmatter + 系统提示词）即可定义自定义 Sub-Agent。
        </p>
      </div>
    </div>
  );
}
