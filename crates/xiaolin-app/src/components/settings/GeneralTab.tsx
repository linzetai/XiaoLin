import { useState, useEffect, useCallback } from "react";
import { useThemeStore, ACCENT_PRESETS, type ThemeMode } from "../../lib/theme";
import * as api from "../../lib/api";
import * as transport from "../../lib/transport";
import { SectionTitle, SettingRow, Toggle, ThemeCard } from "./SettingsShared";
import { NotificationTab } from "./NotificationTab";
import { useConfigStore, type FontSize } from "../../lib/stores/config-store";

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
              className="flex-1 cursor-pointer rounded-[var(--radius-xs)] py-1.5 text-center text-[12px] font-medium transition-all duration-200"
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

      <DisplaySection />

      {transport.isTauri && (
        <div>
          <SectionTitle>系统</SectionTitle>
          <div className="overflow-hidden rounded-[var(--radius-sm)]" style={{ background: "var(--bg-elevated)", border: "0.5px solid var(--separator-opaque)" }}>
            <SettingRow label="开机自启动" description="系统启动时自动运行 XiaoLin，定时任务将正常执行" isLast>
              <Toggle enabled={autostart} onChange={toggleAutostart} />
            </SettingRow>
          </div>
        </div>
      )}
    </div>
  );
}

const THRESHOLD_OPTIONS = [
  { value: 2, label: "2" },
  { value: 3, label: "3" },
  { value: 5, label: "5" },
  { value: 10, label: "10" },
];

const FONT_SIZE_OPTIONS: { value: FontSize; label: string }[] = [
  { value: "small", label: "小" },
  { value: "standard", label: "标准" },
  { value: "large", label: "大" },
  { value: "xlarge", label: "特大" },
];

function DisplaySection() {
  const { display, setDisplayConfig, loadDisplayConfig } = useConfigStore();

  useEffect(() => { loadDisplayConfig(); }, [loadDisplayConfig]);

  return (
    <div>
      <SectionTitle>显示</SectionTitle>
      <div className="overflow-hidden rounded-[var(--radius-sm)]" style={{ background: "var(--bg-elevated)", border: "0.5px solid var(--separator-opaque)" }}>
        <SettingRow label="字体大小" description="调整全局界面文字大小">
          <div className="flex rounded-[var(--radius-xs)] p-0.5" style={{ background: "var(--bg-tertiary)" }}>
            {FONT_SIZE_OPTIONS.map((opt) => (
              <button
                key={opt.value}
                onClick={() => setDisplayConfig({ fontSize: opt.value })}
                className="cursor-pointer rounded-[var(--radius-xs)] px-2.5 py-1 text-center text-[12px] font-medium transition-all duration-200"
                style={{
                  background: display.fontSize === opt.value ? "var(--bg-elevated)" : "transparent",
                  color: display.fontSize === opt.value ? "var(--fill-primary)" : "var(--fill-tertiary)",
                  boxShadow: display.fontSize === opt.value ? "var(--shadow-sm)" : "none",
                }}
              >
                {opt.label}
              </button>
            ))}
          </div>
        </SettingRow>
        <SettingRow label="工具调用折叠阈值" description="连续工具调用达到此数量时自动分组折叠" isLast>
          <div className="flex rounded-[var(--radius-xs)] p-0.5" style={{ background: "var(--bg-tertiary)" }}>
            {THRESHOLD_OPTIONS.map((opt) => (
              <button
                key={opt.value}
                onClick={() => setDisplayConfig({ toolCallGroupThreshold: opt.value })}
                className="cursor-pointer rounded-[var(--radius-xs)] px-2.5 py-1 text-center text-[12px] font-medium transition-all duration-200"
                style={{
                  background: display.toolCallGroupThreshold === opt.value ? "var(--bg-elevated)" : "transparent",
                  color: display.toolCallGroupThreshold === opt.value ? "var(--fill-primary)" : "var(--fill-tertiary)",
                  boxShadow: display.toolCallGroupThreshold === opt.value ? "var(--shadow-sm)" : "none",
                }}
              >
                {opt.label}
              </button>
            ))}
          </div>
        </SettingRow>
      </div>
    </div>
  );
}
