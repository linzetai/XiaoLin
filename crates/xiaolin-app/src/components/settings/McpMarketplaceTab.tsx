import { useState, useMemo } from "react";
import { Search, Download, Trash2, CheckCircle, Package } from "lucide-react";
import { ICON } from "../../lib/ui-tokens";
import mcpRegistry from "../../data/mcp-registry.json";
import { useGatewayStore } from "../../lib/store";

interface McpRegistryEntry {
  id: string;
  name: string;
  description: string;
  category: string;
  command: string;
  args: string[];
  configTemplate: Record<string, unknown>;
}

const CATEGORIES = [
  { id: "all", label: "全部" },
  { id: "development", label: "开发" },
  { id: "productivity", label: "效率" },
  { id: "data", label: "数据" },
  { id: "communication", label: "通讯" },
] as const;

const CATEGORY_COLORS: Record<string, string> = {
  development: "var(--accent)",
  productivity: "var(--semantic-info)",
  data: "var(--semantic-warning)",
  communication: "var(--semantic-success)",
};

export function McpMarketplaceTab() {
  const [search, setSearch] = useState("");
  const [category, setCategory] = useState("all");
  const [installedIds, setInstalledIds] = useState<Set<string>>(new Set());
  const [installing, setInstalling] = useState<string | null>(null);
  const httpUrl = useGatewayStore((s) => s.info?.httpUrl);

  const filtered = useMemo(() => {
    return (mcpRegistry as McpRegistryEntry[]).filter((s) => {
      const matchCategory = category === "all" || s.category === category;
      const matchSearch =
        !search ||
        s.name.toLowerCase().includes(search.toLowerCase()) ||
        s.description.toLowerCase().includes(search.toLowerCase());
      return matchCategory && matchSearch;
    });
  }, [search, category]);

  const handleInstall = async (entry: McpRegistryEntry) => {
    if (!httpUrl || installing) return;
    setInstalling(entry.id);
    try {
      const resp = await fetch(`${httpUrl}/api/admin/mcp-servers`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          action: "add",
          serverId: entry.id,
          config: entry.configTemplate,
        }),
      });
      if (resp.ok) {
        setInstalledIds((prev) => new Set([...prev, entry.id]));
      } else {
        console.error("Install failed:", await resp.text());
      }
    } catch (err) {
      console.error("Install error:", err);
    } finally {
      setInstalling(null);
    }
  };

  const handleUninstall = async (entry: McpRegistryEntry) => {
    if (!httpUrl || installing) return;
    setInstalling(entry.id);
    try {
      const resp = await fetch(`${httpUrl}/api/admin/mcp-servers`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          action: "remove",
          serverId: entry.id,
        }),
      });
      if (resp.ok) {
        setInstalledIds((prev) => {
          const next = new Set(prev);
          next.delete(entry.id);
          return next;
        });
      }
    } catch (err) {
      console.error("Uninstall error:", err);
    } finally {
      setInstalling(null);
    }
  };

  return (
    <div className="flex flex-col gap-4">
      <div className="flex items-center gap-3">
        <div className="relative flex-1">
          <Search {...ICON.sm} className="absolute left-3 top-1/2 -translate-y-1/2" style={{ color: "var(--fill-quaternary)" }} />
          <input
            type="text"
            placeholder="搜索 MCP Server…"
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            className="w-full rounded-[var(--radius-sm)] border py-2 pl-9 pr-3 text-[13px] outline-none"
            style={{
              background: "var(--bg-primary)",
              borderColor: "var(--separator)",
              color: "var(--fill-primary)",
            }}
          />
        </div>
      </div>

      <div className="flex gap-1.5">
        {CATEGORIES.map((c) => (
          <button
            key={c.id}
            onClick={() => setCategory(c.id)}
            className="rounded-full px-3 py-1 text-[12px] font-medium transition-colors"
            style={{
              background: category === c.id ? "var(--accent)" : "var(--bg-tertiary)",
              color: category === c.id ? "white" : "var(--fill-secondary)",
              cursor: "pointer",
              border: "none",
            }}
          >
            {c.label}
          </button>
        ))}
      </div>

      <div className="flex flex-col gap-2">
        {filtered.map((entry) => {
          const isInstalled = installedIds.has(entry.id);
          const isInstalling = installing === entry.id;
          return (
            <div
              key={entry.id}
              className="flex items-center gap-3 rounded-[var(--radius-sm)] p-3 transition-colors"
              style={{
                background: "var(--bg-primary)",
                border: `0.5px solid var(--separator)`,
              }}
            >
              <div
                className="flex h-9 w-9 shrink-0 items-center justify-center rounded-[var(--radius-xs)]"
                style={{
                  background: `${CATEGORY_COLORS[entry.category] ?? "var(--fill-quaternary)"}15`,
                  color: CATEGORY_COLORS[entry.category] ?? "var(--fill-quaternary)",
                }}
              >
                <Package {...ICON.md} />
              </div>
              <div className="min-w-0 flex-1">
                <div className="flex items-center gap-2">
                  <span className="text-[13px] font-semibold" style={{ color: "var(--fill-primary)" }}>
                    {entry.name}
                  </span>
                  <span
                    className="rounded-full px-1.5 py-0.5 text-[10px]"
                    style={{
                      background: `${CATEGORY_COLORS[entry.category] ?? "var(--fill-quaternary)"}15`,
                      color: CATEGORY_COLORS[entry.category] ?? "var(--fill-quaternary)",
                    }}
                  >
                    {CATEGORIES.find((c) => c.id === entry.category)?.label}
                  </span>
                </div>
                <p className="mt-0.5 truncate text-[12px]" style={{ color: "var(--fill-tertiary)" }}>
                  {entry.description}
                </p>
              </div>
              <div className="shrink-0">
                {isInstalled ? (
                  <div className="flex items-center gap-1.5">
                    <CheckCircle {...ICON.sm} style={{ color: "var(--semantic-success)" }} />
                    <button
                      onClick={() => handleUninstall(entry)}
                      disabled={isInstalling}
                      className="flex items-center gap-1 rounded-[var(--radius-xs)] px-2 py-1 text-[12px] transition-colors hover:bg-[var(--bg-hover)]"
                      style={{ color: "var(--semantic-danger)", border: "none", background: "transparent", cursor: "pointer" }}
                    >
                      <Trash2 size={12} />
                      卸载
                    </button>
                  </div>
                ) : (
                  <button
                    onClick={() => handleInstall(entry)}
                    disabled={isInstalling}
                    className="flex items-center gap-1.5 rounded-[var(--radius-sm)] px-3 py-1.5 text-[12px] font-medium transition-colors"
                    style={{
                      background: "var(--accent)",
                      color: "white",
                      border: "none",
                      cursor: isInstalling ? "not-allowed" : "pointer",
                      opacity: isInstalling ? 0.6 : 1,
                    }}
                  >
                    <Download size={12} />
                    {isInstalling ? "安装中…" : "安装"}
                  </button>
                )}
              </div>
            </div>
          );
        })}
        {filtered.length === 0 && (
          <p className="py-8 text-center text-[13px]" style={{ color: "var(--fill-quaternary)" }}>
            没有找到匹配的 MCP Server
          </p>
        )}
      </div>
    </div>
  );
}
