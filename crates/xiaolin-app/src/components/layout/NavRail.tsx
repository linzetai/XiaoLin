import { lazy, Suspense, useState } from "react";
import { MessageSquare, Layout, ListTodo, FolderOpen, Link2, Settings } from "lucide-react";
import { useAgentStore } from "../../lib/agent-store";
import { ClawIcon } from "./ClawIcon";
import { ICON, BTN_ICON } from "../../lib/ui-tokens";
import type { NavItem } from "../../lib/stores/ui-store";

const SettingsPanel = lazy(() =>
  import("../settings/SettingsPanel").then((m) => ({ default: m.SettingsPanel })),
);

interface NavEntry {
  id: NavItem;
  icon: React.ComponentType<{ size?: number; strokeWidth?: number }>;
  label: string;
}

const TOP_ITEMS: NavEntry[] = [
  { id: "chat", icon: MessageSquare, label: "对话" },
  { id: "workspace", icon: Layout, label: "工作室" },
  { id: "tasks", icon: ListTodo, label: "任务" },
  { id: "files", icon: FolderOpen, label: "文件" },
  { id: "connections", icon: Link2, label: "连接" },
];

export function NavRail() {
  const activeNav = useAgentStore((s) => s.activeNav);
  const setActiveNav = useAgentStore((s) => s.setActiveNav);
  const [settingsOpen, setSettingsOpen] = useState(false);

  return (
    <>
      {settingsOpen && (
        <Suspense fallback={null}>
          <SettingsPanel open={settingsOpen} onClose={() => setSettingsOpen(false)} />
        </Suspense>
      )}
      <nav
        className="flex shrink-0 flex-col items-center justify-between py-3"
        style={{
          width: "var(--nav-rail-w)",
          background: "var(--bg-secondary)",
          borderRight: "0.5px solid var(--separator)",
        }}
      >
        <div className="flex flex-col items-center gap-0.5">
          <div
            className="mb-3 flex h-9 w-9 items-center justify-center rounded-[var(--radius-sm)] transition-all duration-150"
            style={{ color: "var(--fill-primary)" }}
          >
            <ClawIcon size={32} />
          </div>

          {TOP_ITEMS.map((item) => {
            const active = activeNav === item.id;
            const Icon = item.icon;
            return (
              <button
                key={item.id}
                onClick={() => setActiveNav(item.id)}
                data-tooltip={item.label}
                className="group relative flex h-9 w-9 items-center justify-center rounded-[var(--radius-sm)] transition-all duration-150"
                style={{
                  background: active ? "var(--tint-bg)" : "transparent",
                  color: active ? "var(--tint)" : "var(--fill-tertiary)",
                }}
              >
                {active && (
                  <span
                    className="absolute left-0 top-1/2 h-4 w-[3px] -translate-y-1/2"
                    style={{
                      background: "var(--tint)",
                      borderRadius: "0 3px 3px 0",
                      animation: "scale-spring var(--duration-normal) var(--ease-spring)",
                    }}
                  />
                )}
                <span className="transition-transform duration-150 group-hover:scale-110">
                  <Icon size={ICON.lg.size} strokeWidth={active ? 1.75 : ICON.lg.strokeWidth} />
                </span>
              </button>
            );
          })}
        </div>

        <div className="flex flex-col items-center gap-1">
          <div className="mb-1 h-px w-6" style={{ background: "var(--separator)" }} />
          <button
            onClick={() => setSettingsOpen(true)}
            className={`${BTN_ICON.lg} hover:scale-105 active:scale-95`}
            style={{ color: "var(--fill-tertiary)" }}
            title="设置"
          >
            <Settings {...ICON.lg} />
          </button>
        </div>
      </nav>
    </>
  );
}
