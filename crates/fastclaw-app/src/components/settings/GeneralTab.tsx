import { useState, useEffect, useCallback } from "react";
import { useThemeStore, ACCENT_PRESETS, type ThemeMode } from "../../lib/theme";
import * as api from "../../lib/api";
import * as transport from "../../lib/transport";
import { SectionTitle, SettingRow, Toggle, ThemeCard } from "./SettingsShared";
import { NotificationTab } from "./NotificationTab";

export function GeneralTab() {
  const { mode, setMode, accent, setAccent, resolved } = useThemeStore();
  const [notifications, setNotifications] = useState(true);
  const [sounds, setSounds] = useState(false);
  const [autoScroll, setAutoScroll] = useState(true);
  const [autostart, setAutostart] = useState(false);
  const [loaded, setLoaded] = useState(false);

  useEffect(() => {
    api.getConfig("session").then((data) => {
      const cfg = data as { key?: string; value?: { autoScroll?: boolean; notifications?: boolean; sounds?: boolean } } | null;
      const val = cfg?.value ?? cfg;
      if (val && typeof val === "object") {
        if ("autoScroll" in val) setAutoScroll(!!(val as Record<string, unknown>).autoScroll);
        if ("notifications" in val) setNotifications(!!(val as Record<string, unknown>).notifications);
        if ("sounds" in val) setSounds(!!(val as Record<string, unknown>).sounds);
      }
      setLoaded(true);
    }).catch(() => setLoaded(true));

    if (transport.isTauri) {
      import("@tauri-apps/plugin-autostart").then(({ isEnabled }) => {
        isEnabled().then(setAutostart).catch(() => {});
      }).catch(() => {});
    }
  }, []);

  const persist = useCallback((key: string, value: boolean) => {
    api.setConfig("session", { [key]: value }).catch(() => {});
  }, []);

  const toggleAutostart = useCallback(async () => {
    try {
      const { enable, disable } = await import("@tauri-apps/plugin-autostart");
      if (autostart) {
        await disable();
        setAutostart(false);
      } else {
        await enable();
        setAutostart(true);
      }
    } catch { /* not available outside Tauri */ }
  }, [autostart]);

  const themeOptions: { value: ThemeMode; label: string }[] = [
    { value: "light", label: "浅色" }, { value: "dark", label: "深色" }, { value: "system", label: "跟随系统" },
  ];

  return (
    <div className="space-y-6">
      <div>
        <SectionTitle>外观</SectionTitle>
        <div className="flex rounded-[var(--radius-xs)] p-0.5" style={{ background: "var(--bg-tertiary)" }}>
          {themeOptions.map((opt) => (
            <button
              key={opt.value}
              onClick={() => setMode(opt.value)}
              className="flex-1 cursor-pointer rounded-[4px] py-1.5 text-center text-[12px] font-medium transition-all duration-200"
              style={{
                background: mode === opt.value ? "var(--bg-elevated)" : "transparent",
                color: mode === opt.value ? "var(--fill-primary)" : "var(--fill-tertiary)",
                boxShadow: mode === opt.value ? "var(--shadow-sm)" : "none",
              }}
            >
              {opt.label}
            </button>
          ))}
        </div>
      </div>

      <div>
        <SectionTitle>主题</SectionTitle>
        <div
          className="overflow-hidden rounded-[var(--radius-sm)] px-5 py-5"
          style={{ background: "var(--bg-elevated)", border: "0.5px solid var(--separator-opaque)" }}
        >
          <div className="grid grid-cols-4 gap-x-3 gap-y-4 justify-items-center">
            {ACCENT_PRESETS.map((preset) => (
              <ThemeCard
                key={preset.id}
                preset={preset}
                selected={accent === preset.id}
                resolved={resolved}
                onClick={() => setAccent(preset.id)}
              />
            ))}
          </div>
        </div>
        <p className="mt-2 text-[11px]" style={{ color: "var(--fill-quaternary)" }}>
          每个主题完整定义背景、文字、强调色，支持浅色与深色模式
        </p>
      </div>

      <NotificationTab
        notifications={notifications}
        setNotifications={setNotifications}
        sounds={sounds}
        setSounds={setSounds}
        autoScroll={autoScroll}
        setAutoScroll={setAutoScroll}
        loaded={loaded}
        persist={persist}
      />
      {transport.isTauri && (
        <div>
          <SectionTitle>系统</SectionTitle>
          <div className="overflow-hidden rounded-[var(--radius-sm)]" style={{ background: "var(--bg-elevated)", border: "0.5px solid var(--separator-opaque)" }}>
            <SettingRow label="开机自启动" description="系统启动时自动运行 FastClaw，定时任务将正常执行" isLast>
              <Toggle enabled={autostart} onChange={toggleAutostart} />
            </SettingRow>
          </div>
        </div>
      )}
    </div>
  );
}
