import { useState, useEffect, lazy, Suspense, useMemo } from "react";
import { useTranslation } from "react-i18next";
import { GearSix, Cube, HardDrives, Info, MagnifyingGlass, Shield, X, ArrowCounterClockwise, Robot, CurrencyDollar } from "@phosphor-icons/react";
import { ICON_SIZE, BTN_ICON } from "../../lib/ui-tokens";

const GeneralTab = lazy(() => import("./GeneralTab").then((m) => ({ default: m.GeneralTab })));
const ModelTab = lazy(() => import("./ModelTab").then((m) => ({ default: m.ModelTab })));
const WebSearchTab = lazy(() => import("./WebSearchTab").then((m) => ({ default: m.WebSearchTab })));
const SubAgentsTab = lazy(() => import("./SubAgentsTab").then((m) => ({ default: m.SubAgentsTab })));
const SecurityTab = lazy(() => import("./SecurityTab").then((m) => ({ default: m.SecurityTab })));
const GatewayTab = lazy(() => import("./GatewayTab").then((m) => ({ default: m.GatewayTab })));
const AboutTab = lazy(() => import("./AboutTab").then((m) => ({ default: m.AboutTab })));
const MigrationTab = lazy(() => import("./MigrationTab").then((m) => ({ default: m.MigrationTab })));
const CostTab = lazy(() => import("../cost/CostDashboard").then((m) => ({ default: m.CostDashboard })));

interface SettingsPanelProps {
  open: boolean;
  onClose: () => void;
}

type SettingsTab = "general" | "models" | "web-search" | "sub-agents" | "security" | "gateway" | "cost" | "about" | "migration";

export function SettingsPanel({ open, onClose }: SettingsPanelProps) {
  const { t } = useTranslation("settings");
  const [tab, setTab] = useState<SettingsTab>("general");

  const tabs: { id: SettingsTab; label: string; icon: React.ReactNode }[] = useMemo(() => [
    { id: "general", label: t("general"), icon: <GearSix size={ICON_SIZE.md} /> },
    { id: "models", label: t("model"), icon: <Cube size={ICON_SIZE.md} /> },
    { id: "web-search", label: t("webSearchTab"), icon: <MagnifyingGlass size={ICON_SIZE.md} /> },
    { id: "sub-agents", label: t("subAgents"), icon: <Robot size={ICON_SIZE.md} /> },
    { id: "security", label: t("security"), icon: <Shield size={ICON_SIZE.md} /> },
    { id: "gateway", label: t("gateway"), icon: <HardDrives size={ICON_SIZE.md} /> },
    { id: "cost", label: t("cost"), icon: <CurrencyDollar size={ICON_SIZE.md} /> },
    { id: "migration", label: t("migration"), icon: <ArrowCounterClockwise size={ICON_SIZE.md} /> },
    { id: "about", label: t("about"), icon: <Info size={ICON_SIZE.md} /> },
  ], [t]);

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
    <div className="fixed inset-0 z-50 flex items-center justify-center">
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
          <div className="mb-2 px-4 text-[12px] font-semibold" style={{ color: "var(--fill-tertiary)" }}>{t("title")}</div>
          {tabs.map((tabItem) => (
            <button
              key={tabItem.id}
              onClick={() => setTab(tabItem.id)}
              className="mx-2 flex cursor-pointer items-center gap-2.5 rounded-[var(--radius-xs)] px-3 py-2 text-left text-[13px] font-medium transition-colors duration-100 hover:bg-[var(--bg-hover)]"
              style={{
                background: tab === tabItem.id ? "var(--bg-active)" : "transparent",
                color: tab === tabItem.id ? "var(--fill-primary)" : "var(--fill-secondary)",
              }}
            >
              {tabItem.icon}
              {tabItem.label}
            </button>
          ))}
        </div>
        <div className="flex min-w-0 flex-1 flex-col">
          <div className="flex shrink-0 items-center justify-between px-6 py-4" style={{ borderBottom: `0.5px solid var(--separator)` }}>
            <h2 className="text-[15px] font-semibold" style={{ color: "var(--fill-primary)" }}>
              {tabs.find((tabItem) => tabItem.id === tab)?.label}
            </h2>
            <button onClick={onClose} className={`${BTN_ICON.sm} cursor-pointer rounded-full`} style={{ color: "var(--fill-tertiary)" }}>
              <X size={ICON_SIZE.md} />
            </button>
          </div>
          <div className="min-w-0 flex-1 overflow-y-auto px-6 py-5">
            <Suspense fallback={<div className="h-full" style={{ background: "var(--bg-elevated)" }} />}>
              <div key={tab} style={{ animation: "tab-crossfade var(--duration-normal) var(--ease-out)" }}>
                {tab === "general" && <GeneralTab />}
                {tab === "models" && <ModelTab />}
                {tab === "web-search" && <WebSearchTab />}
                {tab === "sub-agents" && <SubAgentsTab />}
                {tab === "security" && <SecurityTab />}
                {tab === "gateway" && <GatewayTab />}
                {tab === "cost" && <CostTab />}
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
