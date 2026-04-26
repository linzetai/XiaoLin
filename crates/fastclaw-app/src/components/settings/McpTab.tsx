import { useState, useEffect, useCallback } from "react";
import { Plug, ChevronDown, RefreshCw, Plus, Pencil, Trash2, ToggleLeft, ToggleRight } from "lucide-react";
import * as api from "../../lib/api";
import * as transport from "../../lib/transport";
import { SectionTitle } from "./SettingsShared";
import { McpInstallGuide } from "./McpInstallGuide";


/* ━━━ MCP Tab ━━━ */

interface McpServerEntry {
  id: string;
  command: string;
  args: string[];
  enabled?: boolean;
}

interface McpToolDef {
  name: string;
  description: string;
}

function McpToolsList() {
  const [toolsByServer, setToolsByServer] = useState<Record<string, McpToolDef[]>>({});
  const [expanded, setExpanded] = useState<Set<string>>(new Set());

  useEffect(() => {
    (async () => {
      try {
        const agents = await transport.listAgents();
        const agentId = agents[0]?.agentId;
        if (!agentId) return;
        const tools = await api.listAgentTools(agentId);
        const mcpTools = tools.filter((t) => t.name.startsWith("mcp_"));
        const grouped: Record<string, McpToolDef[]> = {};
        for (const t of mcpTools) {
          const afterPrefix = t.name.slice(4); // strip "mcp_"
          const underIdx = afterPrefix.indexOf("_");
          const serverId = underIdx >= 0 ? afterPrefix.slice(0, underIdx) : afterPrefix;
          if (!grouped[serverId]) grouped[serverId] = [];
          grouped[serverId].push({ name: t.name, description: t.description || "" });
        }
        setToolsByServer(grouped);
      } catch (e) {
        console.warn("[McpToolsList] failed to load tools:", e);
      }
    })();
  }, []);

  const serverIds = Object.keys(toolsByServer);
  if (serverIds.length === 0) return null;

  const toggle = (id: string) => {
    setExpanded((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id); else next.add(id);
      return next;
    });
  };

  return (
    <div className="mt-6 space-y-3">
      <SectionTitle>已连接的 MCP 工具</SectionTitle>
      {serverIds.map((serverId) => {
        const tools = toolsByServer[serverId];
        const isOpen = expanded.has(serverId);
        return (
          <div key={serverId} className="overflow-hidden rounded-[var(--radius-sm)]" style={{ background: "var(--bg-elevated)", border: "0.5px solid var(--separator-opaque)" }}>
            <button onClick={() => toggle(serverId)} className="flex w-full cursor-pointer items-center justify-between px-4 py-2.5 text-left transition-colors duration-100 hover:bg-[var(--bg-hover)]">
              <div className="flex items-center gap-2">
                <Plug size={14} strokeWidth={1.5} style={{ color: "var(--accent)" }} />
                <span className="text-[13px] font-semibold font-mono" style={{ color: "var(--fill-primary)" }}>{serverId}</span>
                <span className="text-[11px] font-mono" style={{ color: "var(--fill-quaternary)" }}>{tools.length} 工具</span>
              </div>
              <ChevronDown size={14} strokeWidth={1.5} style={{ color: "var(--fill-tertiary)", transform: isOpen ? "rotate(180deg)" : "rotate(0)", transition: "transform 0.15s" }} />
            </button>
            {isOpen && (
              <div style={{ borderTop: "0.5px solid var(--separator)" }}>
                {tools.map((t) => {
                  const shortName = t.name.replace(`mcp_${serverId}_`, "");
                  return (
                    <div key={t.name} className="flex items-start gap-2 px-4 py-2" style={{ borderBottom: "0.5px solid var(--separator)" }}>
                      <span className="shrink-0 mt-0.5 text-[12px] font-mono font-medium" style={{ color: "var(--accent)" }}>{shortName}</span>
                      {t.description && <span className="text-[11px] leading-relaxed" style={{ color: "var(--fill-tertiary)" }}>{t.description.length > 100 ? t.description.slice(0, 100) + "…" : t.description}</span>}
                    </div>
                  );
                })}
              </div>
            )}
          </div>
        );
      })}
    </div>
  );
}

function StatusDot({ status }: { status: string }) {
  const color =
    status === "connected" ? "var(--green)" :
    status === "failed" ? "var(--red)" :
    status === "connecting" ? "var(--yellow, #f59e0b)" :
    "var(--fill-quaternary)";
  return (
    <span
      className="inline-block h-2 w-2 shrink-0 rounded-full"
      style={{ background: color, boxShadow: status === "connected" ? `0 0 4px ${color}` : undefined }}
      title={status}
    />
  );
}

export function McpTab() {
  const [servers, setServers] = useState<McpServerEntry[]>([]);
  const [statusMap, setStatusMap] = useState<Record<string, transport.McpServerStatus>>({});
  const [loading, setLoading] = useState(true);
  const [adding, setAdding] = useState(false);
  const [editingId, setEditingId] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [reloading, setReloading] = useState(false);
  const [draft, setDraft] = useState<McpServerEntry>({ id: "", command: "", args: [], enabled: true });

  const loadServers = useCallback(async () => {
    try {
      const resp = await api.getConfig("mcpServers") as { key?: string; value?: McpServerEntry[] } | McpServerEntry[] | null;
      let list: McpServerEntry[] = [];
      if (Array.isArray(resp)) {
        list = resp;
      } else if (resp && typeof resp === "object" && "value" in resp && Array.isArray(resp.value)) {
        list = resp.value;
      }
      setServers(list.map((s) => ({ ...s, enabled: s.enabled !== false })));
    } catch (e) {
      console.warn("[McpTab] failed to load mcpServers:", e);
    } finally {
      setLoading(false);
    }
  }, []);

  const loadStatus = useCallback(async () => {
    try {
      const statuses = await transport.getMcpStatus();
      const map: Record<string, transport.McpServerStatus> = {};
      for (const s of statuses) map[s.id] = s;
      setStatusMap(map);
    } catch (e) {
      console.warn("[McpTab] failed to load MCP status:", e);
    }
  }, []);

  useEffect(() => {
    loadServers().then(loadStatus);
  }, [loadServers, loadStatus]);

  const handleReload = async () => {
    setReloading(true);
    try {
      const statuses = await transport.reloadMcpServers();
      const map: Record<string, transport.McpServerStatus> = {};
      for (const s of statuses) map[s.id] = s;
      setStatusMap(map);
    } catch (e) {
      console.warn("[McpTab] reload failed:", e);
    } finally {
      setReloading(false);
    }
  };

  const persist = async (updated: McpServerEntry[]) => {
    setSaving(true);
    try {
      await api.setConfig("mcpServers", updated);
      setServers(updated);
      // hot reload after save
      const statuses = await transport.reloadMcpServers();
      const map: Record<string, transport.McpServerStatus> = {};
      for (const s of statuses) map[s.id] = s;
      setStatusMap(map);
    } catch (e) {
      console.warn("[McpTab] failed to save mcpServers:", e);
    } finally {
      setSaving(false);
    }
  };

  const handleToggle = (id: string) => {
    const updated = servers.map((s) => s.id === id ? { ...s, enabled: !s.enabled } : s);
    persist(updated);
  };

  const handleDelete = (id: string) => {
    persist(servers.filter((s) => s.id !== id));
  };

  const startAdd = () => {
    setDraft({ id: "", command: "", args: [], enabled: true });
    setAdding(true);
    setEditingId(null);
  };

  const startEdit = (srv: McpServerEntry) => {
    setDraft({ ...srv });
    setEditingId(srv.id);
    setAdding(false);
  };

  const cancelEdit = () => {
    setAdding(false);
    setEditingId(null);
  };

  const saveDraft = () => {
    if (!draft.id.trim() || !draft.command.trim()) return;
    const entry: McpServerEntry = { ...draft, id: draft.id.trim(), command: draft.command.trim() };
    let updated: McpServerEntry[];
    if (editingId) {
      updated = servers.map((s) => s.id === editingId ? entry : s);
    } else {
      if (servers.some((s) => s.id === entry.id)) return;
      updated = [...servers, entry];
    }
    persist(updated);
    setAdding(false);
    setEditingId(null);
  };

  const inputStyle: React.CSSProperties = {
    background: "var(--bg-primary)",
    border: "0.5px solid var(--separator-opaque)",
    borderRadius: "var(--radius-xs)",
    padding: "6px 10px",
    fontSize: 13,
    color: "var(--fill-primary)",
    fontFamily: "var(--font-mono)",
    width: "100%",
    outline: "none",
  };

  const renderForm = () => (
    <div className="space-y-3 rounded-[var(--radius-sm)] p-4" style={{ background: "var(--bg-primary)", border: "0.5px solid var(--separator-opaque)" }}>
      <div>
        <label className="mb-1 block text-[11px] font-medium" style={{ color: "var(--fill-tertiary)" }}>ID</label>
        <input 
          style={inputStyle} 
          value={draft.id} 
          disabled={!!editingId} 
          onChange={(e) => setDraft({ ...draft, id: e.target.value })} 
          placeholder="e.g. chrome-devtools" 
        />
      </div>
      <div>
        <label className="mb-1 block text-[11px] font-medium" style={{ color: "var(--fill-tertiary)" }}>Command</label>
        <input 
          style={inputStyle} 
          value={draft.command} 
          onChange={(e) => setDraft({ ...draft, command: e.target.value })} 
          placeholder="e.g. npx" 
        />
      </div>
      <div>
        <label className="mb-1 block text-[11px] font-medium" style={{ color: "var(--fill-tertiary)" }}>Args (逗号分隔)</label>
        <input 
          style={inputStyle} 
          value={draft.args.join(", ")} 
          onChange={(e) => setDraft({ 
            ...draft, 
            args: e.target.value 
              .split(",") 
              .map((a) => a.trim()) 
              .filter(Boolean) 
          })} 
          placeholder="e.g. -y, @anthropic-ai/chrome-devtools-mcp@latest" 
        />
      </div>
      <div className="flex gap-2 pt-1">
        <button 
          onClick={saveDraft} 
          disabled={!draft.id.trim() || !draft.command.trim()} 
          className="rounded-[var(--radius-xs)] px-3 py-1.5 text-[12px] font-medium transition-colors duration-100 disabled:opacity-50 disabled:cursor-not-allowed"
          style={{ 
            background: "var(--accent)", 
            color: "#fff" 
          }}
        >
          {editingId ? "保存" : "添加"}
        </button>
        <button 
          onClick={cancelEdit} 
          className="rounded-[var(--radius-xs)] px-3 py-1.5 text-[12px] font-medium cursor-pointer transition-colors duration-100 hover:bg-[var(--bg-hover)]"
          style={{ color: "var(--fill-secondary)" }}
        >
          取消
        </button>
      </div>
    </div>
  );

  if (loading) {
    return <div className="flex items-center justify-center py-12 text-[13px]" style={{ color: "var(--fill-tertiary)" }}>加载中…</div>;
  }

  return (
    <div className="space-y-5">
      <div className="flex items-center justify-between">
        <SectionTitle>全局 MCP 服务器</SectionTitle>
        <div className="flex items-center gap-1">
          <button
            onClick={handleReload}
            disabled={reloading}
            className="flex cursor-pointer items-center gap-1.5 rounded-[var(--radius-xs)] px-2.5 py-1 text-[12px] font-medium transition-colors duration-100 hover:bg-[var(--bg-hover)] disabled:opacity-50"
            style={{ color: "var(--fill-secondary)" }}
            title="刷新 MCP 连接"
          >
            <RefreshCw size={13} strokeWidth={1.5} className={reloading ? "animate-spin" : ""} /> 刷新
          </button>
          {!adding && !editingId && (
            <button onClick={startAdd} className="flex cursor-pointer items-center gap-1.5 rounded-[var(--radius-xs)] px-2.5 py-1 text-[12px] font-medium transition-colors duration-100 hover:bg-[var(--bg-hover)]" style={{ color: "var(--accent)" }}>
              <Plus size={13} strokeWidth={2} /> 添加
            </button>
          )}
        </div>
      </div>
      
      <McpInstallGuide />

      {adding && renderForm()}

      {servers.length === 0 && !adding && (
        <div className="rounded-[var(--radius-sm)] py-8 text-center text-[13px]" style={{ color: "var(--fill-tertiary)", background: "var(--bg-elevated)", border: "0.5px solid var(--separator-opaque)" }}>
          暂无全局 MCP 服务器
        </div>
      )}

      {servers.map((srv) => {
        const st = statusMap[srv.id];
        const statusLabel = st?.status === "connected" ? "已连接" : st?.status === "failed" ? "连接失败" : st?.status === "connecting" ? "连接中" : st?.status === "disabled" ? "已禁用" : "未知";
        return editingId === srv.id ? (
          <div key={srv.id}>{renderForm()}</div>
        ) : (
          <div key={srv.id} className="overflow-hidden rounded-[var(--radius-sm)]" style={{ background: "var(--bg-elevated)", border: "0.5px solid var(--separator-opaque)", opacity: srv.enabled ? 1 : 0.55 }}>
            <div className="flex items-center gap-3 px-4 py-3">
              <StatusDot status={st?.status ?? (srv.enabled ? "connecting" : "disabled")} />
              <div className="min-w-0 flex-1">
                <div className="flex items-center gap-2">
                  <span className="text-[13px] font-semibold font-mono" style={{ color: "var(--fill-primary)" }}>{srv.id}</span>
                  <span className="text-[10px] font-medium" style={{ color: st?.status === "connected" ? "var(--green)" : st?.status === "failed" ? "var(--red)" : "var(--fill-quaternary)" }}>
                    {statusLabel}
                    {st?.toolCount ? ` · ${st.toolCount} 工具` : ""}
                  </span>
                </div>
                <div className="mt-0.5 truncate text-[11px] font-mono" style={{ color: "var(--fill-tertiary)" }} title={[srv.command, ...srv.args].join(" ")}>{srv.command} {srv.args.join(" ")}</div>
              </div>
              <div className="flex items-center gap-1">
                {st?.status === "failed" && (
                  <button onClick={handleReload} className="flex h-7 cursor-pointer items-center gap-1 rounded-[var(--radius-xs)] px-1.5 text-[11px] font-medium transition-colors duration-100 hover:bg-[var(--bg-hover)]" style={{ color: "var(--red)" }} title="重试连接">
                    <RefreshCw size={12} strokeWidth={1.5} /> 重试
                  </button>
                )}
                <button onClick={() => handleToggle(srv.id)} className="flex h-7 w-7 cursor-pointer items-center justify-center rounded-full transition-colors duration-100 hover:bg-[var(--bg-hover)]" title={srv.enabled ? "禁用" : "启用"}>
                  {srv.enabled ? <ToggleRight size={16} strokeWidth={1.5} style={{ color: "var(--green)" }} /> : <ToggleLeft size={16} strokeWidth={1.5} style={{ color: "var(--fill-quaternary)" }} />}
                </button>
                <button onClick={() => startEdit(srv)} className="flex h-7 w-7 cursor-pointer items-center justify-center rounded-full transition-colors duration-100 hover:bg-[var(--bg-hover)]" title="编辑">
                  <Pencil size={13} strokeWidth={1.5} style={{ color: "var(--fill-tertiary)" }} />
                </button>
                <button onClick={() => handleDelete(srv.id)} className="flex h-7 w-7 cursor-pointer items-center justify-center rounded-full transition-colors duration-100 hover:bg-[var(--bg-hover)]" title="删除">
                  <Trash2 size={13} strokeWidth={1.5} style={{ color: "var(--red)" }} />
                </button>
              </div>
            </div>
            {st?.status === "failed" && st.error && (
              <div className="border-t px-4 py-2 text-[11px]" style={{ borderColor: "var(--separator)", color: "var(--red)", background: "color-mix(in srgb, var(--red) 5%, transparent)" }}>
                {st.error.length > 200 ? st.error.slice(0, 200) + "…" : st.error}
              </div>
            )}
          </div>
        );
      })}

      {saving && <div className="text-center text-[11px]" style={{ color: "var(--fill-tertiary)" }}>保存中…</div>}

      <p className="text-[11px]" style={{ color: "var(--fill-quaternary)" }}>
        保存配置后自动热重载，无需重启网关
      </p>

      <McpToolsList />
    </div>
  );
}