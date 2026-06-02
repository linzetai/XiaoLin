import { useState, useEffect } from "react";
import { Settings, CheckCircle, XCircle, Play, Square, RotateCcw, Plug } from "lucide-react";
import { ICON } from "../../lib/ui-tokens";

interface McpServerStatus {
  id: string;
  status: "connecting" | "connected" | "failed" | "disabled";
  error?: string | null;
  toolCount: number;
  connectedAt?: string | null;
}

interface McpServerConfig {
  id: string;
  command: string;
  args: string[];
  enabled?: boolean;
}

export const useMcpManager = () => {
  const [servers, setServers] = useState<McpServerConfig[]>([]);
  const [statusMap, setStatusMap] = useState<Record<string, McpServerStatus>>({});
  const [loading, setLoading] = useState(true);
  const [reloading, setReloading] = useState(false);

  // 模拟加载服务器配置
  useEffect(() => {
    // 这里应该从配置文件加载实际的服务器配置
    const mockServers: McpServerConfig[] = [
      { id: "chrome-devtools", command: "npx", args: ["@modelcontextprotocol/chrome-devtools-mcp"], enabled: true },
      { id: "github", command: "npx", args: ["@modelcontextprotocol/github-mcp"], enabled: false },
    ];
    setServers(mockServers);
    setLoading(false);
  }, []);

  // 模拟加载服务器状态
  const loadStatus = async () => {
    setReloading(true);
    // 这里应该实际查询MCP服务器状态
    const mockStatus: McpServerStatus[] = [
      { id: "chrome-devtools", status: "connected", toolCount: 5, connectedAt: new Date().toISOString() },
      { id: "github", status: "failed", error: "找不到命令", toolCount: 0 },
    ];
    const map: Record<string, McpServerStatus> = {};
    for (const s of mockStatus) map[s.id] = s;
    setStatusMap(map);
    setReloading(false);
  };

  useEffect(() => {
    loadStatus();
  }, []);

  const reloadServers = async () => {
    await loadStatus();
  };

  const addServer = (server: McpServerConfig) => {
    setServers([...servers, server]);
  };

  const removeServer = (id: string) => {
    setServers(servers.filter(s => s.id !== id));
  };

  const updateServer = (id: string, updates: Partial<McpServerConfig>) => {
    setServers(servers.map(s => s.id === id ? { ...s, ...updates } : s));
  };

  const toggleServer = (id: string) => {
    const server = servers.find(s => s.id === id);
    if (server) {
      updateServer(id, { enabled: !server.enabled });
    }
  };

  return {
    servers,
    statusMap,
    loading,
    reloading,
    reloadServers,
    addServer,
    removeServer,
    updateServer,
    toggleServer
  };
};

export const McpServerCard = ({ 
  server, 
  status, 
  onToggle, 
  onEdit, 
  onDelete 
}: { 
  server: McpServerConfig; 
  status: McpServerStatus; 
  onToggle: (id: string) => void; 
  onEdit: (server: McpServerConfig) => void; 
  onDelete: (id: string) => void; 
}) => {
  const statusColors = {
    connected: "var(--green)",
    failed: "var(--red)",
    connecting: "var(--yellow)",
    disabled: "var(--fill-quaternary)"
  };

  const statusIcons = {
    connected: <CheckCircle {...ICON.sm} style={{ color: "var(--green)" }} />,
    failed: <XCircle {...ICON.sm} style={{ color: "var(--red)" }} />,
    connecting: <RotateCcw {...ICON.sm} className="animate-spin" style={{ color: "var(--yellow)" }} />,
    disabled: <XCircle {...ICON.sm} style={{ color: "var(--fill-quaternary)" }} />
  };

  return (
    <div 
      className="overflow-hidden rounded-[var(--radius-sm)] transition-all duration-200 hover:shadow-[0_0_0_1px_var(--separator)]"
      style={{ 
        background: "var(--bg-elevated)", 
        border: "0.5px solid var(--separator-opaque)",
        opacity: server.enabled ? 1 : 0.65
      }}
    >
      <div className="flex items-center gap-3 px-4 py-3">
        <div className="flex h-8 w-8 items-center justify-center rounded-full" style={{ background: "color-mix(in srgb, var(--accent) 15%, transparent)" }}>
          <Plug {...ICON.md} style={{ color: "var(--accent)" }} />
        </div>
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            <span className="truncate text-[14px] font-semibold font-mono" style={{ color: "var(--fill-primary)" }}>
              {server.id}
            </span>
            <div className="flex items-center gap-1">
              {statusIcons[status.status]}
              <span 
                className="text-[11px] font-medium" 
                style={{ color: statusColors[status.status] }}
              >
                {status.status === "connected" ? "已连接" : 
                 status.status === "failed" ? "连接失败" : 
                 status.status === "connecting" ? "连接中" : "已禁用"}
              </span>
              {status.toolCount > 0 && (
                <span className="text-[10px] font-mono" style={{ color: "var(--fill-tertiary)" }}>
                  · {status.toolCount} 工具
                </span>
              )}
            </div>
          </div>
          <div className="mt-1 truncate text-[12px] font-mono" style={{ color: "var(--fill-tertiary)" }}>
            {server.command} {server.args.join(" ")}
          </div>
        </div>
        <div className="flex items-center gap-1">
          <button 
            onClick={() => onToggle(server.id)}
            className="flex h-8 w-8 cursor-pointer items-center justify-center rounded-full transition-colors duration-100 hover:bg-[var(--bg-hover)]"
            title={server.enabled ? "禁用服务器" : "启用服务器"}
          >
            {server.enabled ? (
              <Square {...ICON.md} style={{ color: "var(--red)" }} />
            ) : (
              <Play {...ICON.md} style={{ color: "var(--green)" }} />
            )}
          </button>
          <button 
            onClick={() => onEdit(server)}
            className="flex h-8 w-8 cursor-pointer items-center justify-center rounded-full transition-colors duration-100 hover:bg-[var(--bg-hover)]"
            title="编辑服务器配置"
          >
            <Settings {...ICON.sm} style={{ color: "var(--fill-tertiary)" }} />
          </button>
          <button 
            onClick={() => onDelete(server.id)}
            className="flex h-8 w-8 cursor-pointer items-center justify-center rounded-full transition-colors duration-100 hover:bg-[var(--bg-hover)]"
            title="删除服务器"
          >
            <XCircle {...ICON.sm} style={{ color: "var(--red)" }} />
          </button>
        </div>
      </div>
      {status.status === "failed" && status.error && (
        <div 
          className="border-t px-4 py-2 text-[11px]"
          style={{ 
            borderColor: "var(--separator)", 
            color: "var(--red)", 
            background: "color-mix(in srgb, var(--red) 5%, transparent)" 
          }}
        >
          错误: {status.error}
        </div>
      )}
    </div>
  );
};

export const McpServerForm = ({ 
  server, 
  onSave, 
  onCancel, 
  onChange 
}: { 
  server?: McpServerConfig; 
  onSave: (server: McpServerConfig) => void; 
  onCancel: () => void; 
  onChange: (field: keyof McpServerConfig, value: any) => void; 
}) => {
  const isNew = !server;
  
  const handleSave = () => {
    if (server && server.id && server.command) {
      onSave(server);
    }
  };

  const inputStyle: React.CSSProperties = {
    background: "var(--bg-base)",
    border: "0.5px solid var(--separator-opaque)",
    borderRadius: "var(--radius-xs)",
    padding: "8px 12px",
    fontSize: 13,
    color: "var(--fill-primary)",
    fontFamily: "var(--font-mono)",
    width: "100%",
    outline: "none",
  };

  return (
    <div className="space-y-4 rounded-[var(--radius-sm)] p-5" style={{ background: "var(--bg-primary)", border: "0.5px solid var(--separator-opaque)" }}>
      <h3 className="text-[15px] font-semibold" style={{ color: "var(--fill-primary)" }}>
        {isNew ? "添加 MCP 服务器" : "编辑 MCP 服务器"}
      </h3>
      
      <div>
        <label className="mb-1 block text-[12px] font-medium" style={{ color: "var(--fill-tertiary)" }}>服务器 ID</label>
        <input
          style={inputStyle}
          value={server?.id || ""}
          onChange={(e) => onChange('id', e.target.value)}
          placeholder="例如：chrome-devtools"
          disabled={!isNew}  // ID 在编辑时不可更改
        />
        <p className="mt-1 text-[11px]" style={{ color: "var(--fill-tertiary)" }}>
          服务器的唯一标识符，只能包含字母、数字和连字符
        </p>
      </div>
      
      <div>
        <label className="mb-1 block text-[12px] font-medium" style={{ color: "var(--fill-tertiary)" }}>命令</label>
        <input
          style={inputStyle}
          value={server?.command || ""}
          onChange={(e) => onChange('command', e.target.value)}
          placeholder="例如：npx 或 node"
        />
        <p className="mt-1 text-[11px]" style={{ color: "var(--fill-tertiary)" }}>
          启动 MCP 服务器的命令
        </p>
      </div>
      
      <div>
        <label className="mb-1 block text-[12px] font-medium" style={{ color: "var(--fill-tertiary)" }}>参数</label>
        <input
          style={inputStyle}
          value={server?.args.join(" ") || ""}
          onChange={(e) => onChange('args', e.target.value.split(" ").filter(arg => arg.trim() !== ""))}
          placeholder="例如：@modelcontextprotocol/chrome-devtools-mcp@latest"
        />
        <p className="mt-1 text-[11px]" style={{ color: "var(--fill-tertiary)" }}>
          传递给命令的参数，用空格分隔
        </p>
      </div>
      
      <div className="flex gap-3 pt-2">
        <button
          onClick={handleSave}
          disabled={!server?.id?.trim() || !server?.command?.trim()}
          className="rounded-[var(--radius-sm)] px-4 py-2 text-[13px] font-medium transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
          style={{ background: "var(--accent)", color: "white" }}
        >
          {isNew ? "添加服务器" : "保存更改"}
        </button>
        <button
          onClick={onCancel}
          className="rounded-[var(--radius-sm)] px-4 py-2 text-[13px] font-medium transition-colors hover:bg-[var(--bg-hover)]"
          style={{ color: "var(--fill-secondary)" }}
        >
          取消
        </button>
      </div>
    </div>
  );
};