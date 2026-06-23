import { useState, useEffect, useCallback, useMemo } from "react";
import { useTranslation } from "react-i18next";
import { useThemeStore, ACCENT_PRESETS, type ThemeMode } from "../../lib/theme";
import * as api from "../../lib/api";
import * as transport from "../../lib/transport";
import { SectionTitle, SettingRow, Toggle, ThemeCard } from "./SettingsShared";
import { NotificationTab } from "./NotificationTab";
import { useConfigStore, type FontSize } from "../../lib/stores/config-store";
import { useLocaleStore, type Locale, type ResponseLang } from "../../lib/stores/locale-store";

export function GeneralTab() {
  const { t } = useTranslation("settings");
  const mode = useThemeStore((s) => s.mode);
  const setMode = useThemeStore((s) => s.setMode);
  const accent = useThemeStore((s) => s.accent);
  const setAccent = useThemeStore((s) => s.setAccent);
  const resolved = useThemeStore((s) => s.resolved);
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

  const themeOptions: { value: ThemeMode; label: string }[] = useMemo(() => [
    { value: "light", label: t("themeMode_light") },
    { value: "dark", label: t("themeMode_dark") },
    { value: "system", label: t("themeMode_system") },
  ], [t]);

  return (
    <div className="space-y-6">
      <div>
        <SectionTitle>{t("appearance")}</SectionTitle>
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
        <SectionTitle>{t("themeSection")}</SectionTitle>
        <div
          className="overflow-hidden rounded-[var(--radius-sm)] px-5 py-5"
          style={{ background: "var(--bg-elevated)", border: "0.5px solid var(--separator-opaque)" }}
        >
          <div className="grid grid-cols-4 gap-x-3 gap-y-4 justify-items-center">
            {ACCENT_PRESETS.map((preset) => (
              <ThemeCard
                key={preset.id}
                preset={preset}
                label={t(`theme_${preset.id}`)}
                selected={accent === preset.id}
                resolved={resolved}
                onClick={() => setAccent(preset.id)}
              />
            ))}
          </div>
        </div>
        <p className="mt-2 text-[11px]" style={{ color: "var(--fill-quaternary)" }}>
          {t("themeHint")}
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

      <LanguageSection />

      {transport.isTauri && (
        <div>
          <SectionTitle>{t("systemSection")}</SectionTitle>
          <div className="overflow-hidden rounded-[var(--radius-sm)]" style={{ background: "var(--bg-elevated)", border: "0.5px solid var(--separator-opaque)" }}>
            <SettingRow label={t("autostart")} description={t("autostartDesc")} isLast>
              <Toggle enabled={autostart} onChange={toggleAutostart} />
            </SettingRow>
          </div>
        </div>
      )}

      <SkillExtractionSection />
    </div>
  );
}

const LOCALE_OPTIONS: { value: Locale; label: string }[] = [
  { value: "zh", label: "中文" },
  { value: "en", label: "English" },
];

const RESPONSE_LANG_OPTIONS: { value: ResponseLang; labelKey: string }[] = [
  { value: "zh", labelKey: "responseLang_zh" },
  { value: "en", labelKey: "responseLang_en" },
  { value: "follow-ui", labelKey: "responseLang_followUi" },
  { value: "auto", labelKey: "responseLang_auto" },
];

function LanguageSection() {
  const { t } = useTranslation("settings");
  const locale = useLocaleStore((s) => s.locale);
  const responseLang = useLocaleStore((s) => s.responseLang);
  const setLocale = useLocaleStore((s) => s.setLocale);
  const setResponseLang = useLocaleStore((s) => s.setResponseLang);

  return (
    <div>
      <SectionTitle>{t("language")}</SectionTitle>
      <div className="overflow-hidden rounded-[var(--radius-sm)]" style={{ background: "var(--bg-elevated)", border: "0.5px solid var(--separator-opaque)" }}>
        <SettingRow label={t("uiLanguage")}>
          <div className="flex rounded-[var(--radius-xs)] p-0.5" style={{ background: "var(--bg-tertiary)" }}>
            {LOCALE_OPTIONS.map((opt) => (
              <button
                key={opt.value}
                onClick={() => setLocale(opt.value)}
                className="cursor-pointer rounded-[var(--radius-xs)] px-3 py-1 text-center text-[12px] font-medium transition-all duration-200"
                style={{
                  background: locale === opt.value ? "var(--bg-elevated)" : "transparent",
                  color: locale === opt.value ? "var(--fill-primary)" : "var(--fill-tertiary)",
                  boxShadow: locale === opt.value ? "var(--shadow-sm)" : "none",
                }}
              >
                {opt.label}
              </button>
            ))}
          </div>
        </SettingRow>
        <SettingRow label={t("responseLanguage")} isLast>
          <div className="flex rounded-[var(--radius-xs)] p-0.5" style={{ background: "var(--bg-tertiary)" }}>
            {RESPONSE_LANG_OPTIONS.map((opt) => (
              <button
                key={opt.value}
                onClick={() => setResponseLang(opt.value)}
                className="cursor-pointer rounded-[var(--radius-xs)] px-2.5 py-1 text-center text-[12px] font-medium transition-all duration-200"
                style={{
                  background: responseLang === opt.value ? "var(--bg-elevated)" : "transparent",
                  color: responseLang === opt.value ? "var(--fill-primary)" : "var(--fill-tertiary)",
                  boxShadow: responseLang === opt.value ? "var(--shadow-sm)" : "none",
                }}
              >
                {t(opt.labelKey)}
              </button>
            ))}
          </div>
        </SettingRow>
      </div>
    </div>
  );
}

const THRESHOLD_OPTIONS = [
  { value: 2, label: "2" },
  { value: 3, label: "3" },
  { value: 5, label: "5" },
  { value: 10, label: "10" },
];

function DisplaySection() {
  const { t } = useTranslation("settings");
  const display = useConfigStore((s) => s.display);
  const setDisplayConfig = useConfigStore((s) => s.setDisplayConfig);
  const loadDisplayConfig = useConfigStore((s) => s.loadDisplayConfig);

  const fontSizeOptions: { value: FontSize; label: string }[] = useMemo(() => [
    { value: "small", label: t("fontSize_small") },
    { value: "standard", label: t("fontSize_standard") },
    { value: "large", label: t("fontSize_large") },
    { value: "xlarge", label: t("fontSize_xlarge") },
  ], [t]);

  useEffect(() => { loadDisplayConfig(); }, [loadDisplayConfig]);

  return (
    <div>
      <SectionTitle>{t("displaySection")}</SectionTitle>
      <div className="overflow-hidden rounded-[var(--radius-sm)]" style={{ background: "var(--bg-elevated)", border: "0.5px solid var(--separator-opaque)" }}>
        <SettingRow label={t("fontSize")} description={t("fontSizeDesc")}>
          <div className="flex rounded-[var(--radius-xs)] p-0.5" style={{ background: "var(--bg-tertiary)" }}>
            {fontSizeOptions.map((opt) => (
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
        <SettingRow label={t("lineNumbers")} description={t("lineNumbersDesc")}>
          <Toggle enabled={display.showLineNumbers} onChange={() => setDisplayConfig({ showLineNumbers: !display.showLineNumbers })} />
        </SettingRow>
        <SettingRow label={t("chatLinkTarget")} description={t("chatLinkTargetDesc")}>
          <div className="flex rounded-[var(--radius-xs)] p-0.5" style={{ background: "var(--bg-tertiary)" }}>
            {([
              { value: "builtin" as const, label: t("chatLinkTarget_builtin") },
              { value: "external" as const, label: t("chatLinkTarget_external") },
            ]).map((opt) => (
              <button
                key={opt.value}
                onClick={() => setDisplayConfig({ chatLinkTarget: opt.value })}
                className="cursor-pointer rounded-[var(--radius-xs)] px-2.5 py-1 text-center text-[12px] font-medium transition-all duration-200"
                style={{
                  background: display.chatLinkTarget === opt.value ? "var(--bg-elevated)" : "transparent",
                  color: display.chatLinkTarget === opt.value ? "var(--fill-primary)" : "var(--fill-tertiary)",
                  boxShadow: display.chatLinkTarget === opt.value ? "var(--shadow-sm)" : "none",
                }}
              >
                {opt.label}
              </button>
            ))}
          </div>
        </SettingRow>
        <SettingRow label={t("toolCallThreshold")} description={t("toolCallThresholdDesc")} isLast>
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

function SkillExtractionSection() {
  const { t } = useTranslation("settings");
  const [enabled, setEnabled] = useState(false);
  const [model, setModel] = useState("");
  const [dailyLimit, setDailyLimit] = useState(50);
  const [loaded, setLoaded] = useState(false);

  useEffect(() => {
    api.getConfig("evolution").then((data) => {
      const cfg = (data as { value?: Record<string, unknown> })?.value ?? data;
      if (cfg && typeof cfg === "object") {
        const c = cfg as Record<string, unknown>;
        if ("skillExtractionEnabled" in c) setEnabled(!!c.skillExtractionEnabled);
        if ("skillExtractionModel" in c && typeof c.skillExtractionModel === "string") setModel(c.skillExtractionModel);
        if ("skillExtractionDailyLimit" in c && typeof c.skillExtractionDailyLimit === "number") setDailyLimit(c.skillExtractionDailyLimit);
      }
      setLoaded(true);
    }).catch(() => setLoaded(true));
  }, []);

  const saveEnabled = useCallback(async (val: boolean) => {
    setEnabled(val);
    await api.setConfig("evolution.skillExtractionEnabled", val);
  }, []);

  const saveModel = useCallback(async (val: string) => {
    setModel(val);
    await api.setConfig("evolution.skillExtractionModel", val || null);
  }, []);

  const saveLimit = useCallback(async (val: number) => {
    const n = Math.max(1, Math.min(500, val));
    setDailyLimit(n);
    await api.setConfig("evolution.skillExtractionDailyLimit", n);
  }, []);

  if (!loaded) return null;

  return (
    <div>
      <SectionTitle>{t("skillExtraction", "技能学习")}</SectionTitle>
      <div className="overflow-hidden rounded-[var(--radius-sm)]" style={{ background: "var(--bg-elevated)", border: "0.5px solid var(--separator-opaque)" }}>
        <SettingRow
          label={t("skillExtractionEnabled", "自动提取技能")}
          description={t("skillExtractionEnabledDesc", "从对话历史中自动学习使用模式（消耗 LLM 额度）")}
        >
          <Toggle enabled={enabled} onChange={() => saveEnabled(!enabled)} />
        </SettingRow>
        {enabled && (
          <>
            <SettingRow
              label={t("skillExtractionModel", "提取模型")}
              description={t("skillExtractionModelDesc", "留空则使用系统默认模型")}
            >
              <input
                type="text"
                value={model}
                onChange={(e) => setModel(e.target.value)}
                onBlur={() => saveModel(model)}
                placeholder="deepseek/deepseek-v4-flash"
                className="w-[180px] rounded-[var(--radius-xs)] border px-2 py-1 text-[12px]"
                style={{
                  borderColor: "var(--separator)",
                  background: "var(--bg-tertiary)",
                  color: "var(--fill-primary)",
                }}
              />
            </SettingRow>
            <SettingRow
              label={t("skillExtractionDailyLimit", "每日上限")}
              description={t("skillExtractionDailyLimitDesc", "每天最多 LLM 调用次数")}
              isLast
            >
              <input
                type="number"
                value={dailyLimit}
                onChange={(e) => setDailyLimit(Number(e.target.value))}
                onBlur={() => saveLimit(dailyLimit)}
                min={1}
                max={500}
                className="w-[80px] rounded-[var(--radius-xs)] border px-2 py-1 text-center text-[12px]"
                style={{
                  borderColor: "var(--separator)",
                  background: "var(--bg-tertiary)",
                  color: "var(--fill-primary)",
                }}
              />
            </SettingRow>
          </>
        )}
        {!enabled && (
          <SettingRow
            label=""
            description={t("skillExtractionDisabledHint", "开启后将定期从对话历史中提取可复用的操作模式")}
            isLast
          >
            <span />
          </SettingRow>
        )}
      </div>
    </div>
  );
}
