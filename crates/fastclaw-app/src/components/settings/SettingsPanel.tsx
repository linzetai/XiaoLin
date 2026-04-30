import { useState, useEffect, lazy, Suspense } from "react";
import { Settings2, Box, Wrench, Server, Info, Search, Shield, Plug, X, RotateCcw } from "lucide-react";

const GeneralTab = lazy(() => import("./GeneralTab").then((m) => ({ default: m.GeneralTab })));
const ModelTab = lazy(() => import("./ModelTab").then((m) => ({ default: m.ModelTab })));
const WebSearchTab = lazy(() => import("./WebSearchTab").then((m) => ({ default: m.WebSearchTab })));
const SkillsTab = lazy(() => import("./SkillsTab").then((m) => ({ default: m.SkillsTab })));
const McpTab = lazy(() => import("./McpTab").then((m) => ({ default: m.McpTab })));
const SecurityTab = lazy(() => import("./SecurityTab").then((m) => ({ default: m.SecurityTab })));
const GatewayTab = lazy(() => import("./GatewayTab").then((m) => ({ default: m.GatewayTab })));
const AboutTab = lazy(() => import("./AboutTab").then((m) => ({ default: m.AboutTab })));
const MigrationTab = lazy(() => import("./MigrationTab").then((m) => ({ default: m.MigrationTab })));

interface SettingsPanelProps {
  open: boolean;
  onClose: () => void;
}

type SettingsTab = "general" | "models" | "web-search" | "skills" | "mcp" | "security" | "gateway" | "about" | "migration";

const ICON_PROPS = { size: 16, strokeWidth: 1.5 } as const;
const TABS: { id: SettingsTab; label: string; icon: React.ReactNode }[] = [
  { id: "general", label: "通用", icon: <Settings2 {...ICON_PROPS} /> },
  { id: "models", label: "模型", icon: <Box {...ICON_PROPS} /> },
  { id: "web-search", label: "联网搜索", icon: <Search {...ICON_PROPS} /> },
  { id: "skills", label: "Skills", icon: <Wrench {...ICON_PROPS} /> },
  { id: "mcp", label: "MCP", icon: <Plug {...ICON_PROPS} /> },
  { id: "security", label: "安全", icon: <Shield {...ICON_PROPS} /> },
  { id: "gateway", label: "网关", icon: <Server {...ICON_PROPS} /> },
  { id: "migration", label: "迁移", icon: <RotateCcw {...ICON_PROPS} /> },
  { id: "about", label: "关于", icon: <Info {...ICON_PROPS} /> },
];

export function SettingsPanel({ open, onClose }: SettingsPanelProps) {
  const [tab, setTab] = useState<SettingsTab>("general");

  useEffect(() => {
    if (!open) return;
    const handleKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") { e.stopPropagation(); onClose(); }
    };
    window.addEventListener("keydown", handleKey);
    return () => window.removeEventListener("keydown", handleKey);
  }, [open, onClose]);

  if (!open) return null;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center" style={{ animation: "fade-in var(--duration-fast) var(--ease-out)" }}>
      <div className="absolute inset-0" style={{ background: "rgba(0, 0, 0, 0.3)" }} onClick={onClose} />
      <div
        className="relative flex overflow-hidden rounded-[var(--radius-xl)]"
        style={{
          width: "min(720px, calc(100vw - 64px))",
          height: "min(520px, calc(100vh - 80px))",
          background: "var(--bg-elevated)",
          boxShadow: "var(--shadow-lg)",
          animation: "scale-in var(--duration-normal) var(--ease-out)",
          border: `0.5px solid var(--separator)`,
        }}
      >
        <div className="flex w-[160px] shrink-0 flex-col py-3" style={{ background: "var(--bg-secondary)", borderRight: `0.5px solid var(--separator)` }}>
          <div className="mb-2 px-4 text-[12px] font-semibold" style={{ color: "var(--fill-tertiary)" }}>设置</div>
          {TABS.map((t) => (
            <button
              key={t.id}
              onClick={() => setTab(t.id)}
              className="mx-2 flex cursor-pointer items-center gap-2.5 rounded-[var(--radius-xs)] px-3 py-2 text-left text-[13px] font-medium transition-colors duration-100 hover:bg-[var(--bg-hover)]"
              style={{
                background: tab === t.id ? "var(--bg-active)" : "transparent",
                color: tab === t.id ? "var(--fill-primary)" : "var(--fill-secondary)",
              }}
            >
              {t.icon}
              {t.label}
            </button>
          ))}
        </div>
        <div className="flex flex-1 flex-col">
          <div className="flex shrink-0 items-center justify-between px-6 py-4" style={{ borderBottom: `0.5px solid var(--separator)` }}>
            <h2 className="text-[15px] font-semibold" style={{ color: "var(--fill-primary)" }}>
              {TABS.find((t) => t.id === tab)?.label}
            </h2>
            <button onClick={onClose} className="flex h-7 w-7 cursor-pointer items-center justify-center rounded-full transition-colors duration-100 hover:bg-[var(--bg-hover)]" style={{ color: "var(--fill-tertiary)" }}>
              <X size={14} strokeWidth={1.5} />
            </button>
          </div>
          <div className="flex-1 overflow-y-auto px-6 py-5">
            <Suspense fallback={<div className="h-full" style={{ background: "var(--bg-elevated)" }} />}>
              {tab === "general" && <GeneralTab />}
              {tab === "models" && <ModelTab />}
              {tab === "web-search" && <WebSearchTab />}
              {tab === "skills" && <SkillsTab />}
              {tab === "mcp" && <McpTab />}
              {tab === "security" && <SecurityTab />}
              {tab === "gateway" && <GatewayTab />}
              {tab === "migration" && <MigrationTab />}
              {tab === "about" && <AboutTab />}
            </Suspense>
          </div>
        </div>
      </div>
    </div>
  );
}
