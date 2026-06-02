import { useState, useEffect, lazy, Suspense } from "react";
import { Settings2, Box, Wrench, Server, Info, Search, Shield, X, RotateCcw, Bot } from "lucide-react";
import { ICON, BTN_ICON } from "../../lib/ui-tokens";

const GeneralTab = lazy(() => import("./GeneralTab").then((m) => ({ default: m.GeneralTab })));
const ModelTab = lazy(() => import("./ModelTab").then((m) => ({ default: m.ModelTab })));
const WebSearchTab = lazy(() => import("./WebSearchTab").then((m) => ({ default: m.WebSearchTab })));
const SkillsTab = lazy(() => import("./SkillsTab").then((m) => ({ default: m.SkillsTab })));
const SubAgentsTab = lazy(() => import("./SubAgentsTab").then((m) => ({ default: m.SubAgentsTab })));
const SecurityTab = lazy(() => import("./SecurityTab").then((m) => ({ default: m.SecurityTab })));
const GatewayTab = lazy(() => import("./GatewayTab").then((m) => ({ default: m.GatewayTab })));
const AboutTab = lazy(() => import("./AboutTab").then((m) => ({ default: m.AboutTab })));
const MigrationTab = lazy(() => import("./MigrationTab").then((m) => ({ default: m.MigrationTab })));

interface SettingsPanelProps {
  open: boolean;
  onClose: () => void;
}

type SettingsTab = "general" | "models" | "web-search" | "skills" | "sub-agents" | "security" | "gateway" | "about" | "migration";

const TABS: { id: SettingsTab; label: string; icon: React.ReactNode }[] = [
  { id: "general", label: "通用", icon: <Settings2 {...ICON.md} /> },
  { id: "models", label: "模型", icon: <Box {...ICON.md} /> },
  { id: "web-search", label: "联网搜索", icon: <Search {...ICON.md} /> },
  { id: "skills", label: "Skills", icon: <Wrench {...ICON.md} /> },
  { id: "sub-agents", label: "Sub-Agents", icon: <Bot {...ICON.md} /> },
  { id: "security", label: "安全", icon: <Shield {...ICON.md} /> },
  { id: "gateway", label: "网关", icon: <Server {...ICON.md} /> },
  { id: "migration", label: "迁移", icon: <RotateCcw {...ICON.md} /> },
  { id: "about", label: "关于", icon: <Info {...ICON.md} /> },
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
      <div
        className="absolute inset-0"
        style={{
          background: "rgba(0, 0, 0, 0.25)",
          backdropFilter: "blur(4px)",
          WebkitBackdropFilter: "blur(4px)",
        }}
        onClick={onClose}
      />
      <div
        className="relative flex overflow-hidden rounded-[var(--radius-xl)]"
        style={{
          width: "min(720px, calc(100vw - 64px))",
          height: "min(520px, calc(100vh - 80px))",
          background: "var(--bg-elevated)",
          boxShadow: "var(--shadow-lg)",
          animation: "scale-spring-lg var(--duration-slow) var(--ease-spring)",
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
        <div className="flex min-w-0 flex-1 flex-col">
          <div className="flex shrink-0 items-center justify-between px-6 py-4" style={{ borderBottom: `0.5px solid var(--separator)` }}>
            <h2 className="text-[15px] font-semibold" style={{ color: "var(--fill-primary)" }}>
              {TABS.find((t) => t.id === tab)?.label}
            </h2>
            <button onClick={onClose} className={`${BTN_ICON.sm} cursor-pointer rounded-full`} style={{ color: "var(--fill-tertiary)" }}>
              <X {...ICON.md} />
            </button>
          </div>
          <div className="min-w-0 flex-1 overflow-y-auto px-6 py-5">
            <Suspense fallback={<div className="h-full" style={{ background: "var(--bg-elevated)" }} />}>
              <div key={tab} style={{ animation: "tab-crossfade var(--duration-normal) var(--ease-out)" }}>
                {tab === "general" && <GeneralTab />}
                {tab === "models" && <ModelTab />}
                {tab === "web-search" && <WebSearchTab />}
                {tab === "skills" && <SkillsTab />}
                {tab === "sub-agents" && <SubAgentsTab />}
                {tab === "security" && <SecurityTab />}
                {tab === "gateway" && <GatewayTab />}
                {tab === "migration" && <MigrationTab />}
                {tab === "about" && <AboutTab />}
              </div>
            </Suspense>
          </div>
        </div>
      </div>
    </div>
  );
}
