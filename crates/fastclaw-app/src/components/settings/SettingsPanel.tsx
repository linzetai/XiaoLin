import { useState, useEffect, useCallback, useMemo } from "react";
import { useThemeStore, ACCENT_PRESETS, type ThemeMode } from "../../lib/theme";
import { useGatewayStore } from "../../lib/store";
import { Settings2, Box, Wrench, Server, Info, ChevronDown, Plus, Pencil, Globe, User, X, RefreshCw, Upload, FolderOpen, FileText, Plug, Trash2, ToggleLeft, ToggleRight, Eye, EyeOff, Zap, CheckCircle, XCircle, Loader2, Search, Shield, Check } from "lucide-react";
import { ClawIcon } from "../layout/ClawIcon";
import * as api from "../../lib/api";
import * as transport from "../../lib/transport";

interface SettingsPanelProps {
  open: boolean;
  onClose: () => void;
}

type SettingsTab = "general" | "models" | "web-search" | "skills" | "mcp" | "security" | "gateway" | "about";

/* ━━━ Shared UI ━━━ */

function SectionTitle({ children }: { children: React.ReactNode }) {
  return (
    <h3 className="mb-3 text-[11px] font-semibold uppercase tracking-wider" style={{ color: "var(--fill-tertiary)" }}>
      {children}
    </h3>
  );
}

function SettingRow({ label, description, children, isLast }: { label: string; description?: string; children: React.ReactNode; isLast?: boolean }) {
  return (
    <div
      className="flex items-center justify-between px-4 py-3"
      style={!isLast ? { borderBottom: "0.5px solid var(--separator)" } : undefined}
    >
      <div className="mr-4 min-w-0 flex-1">
        <div className="text-[13px] font-medium" style={{ color: "var(--fill-primary)" }}>{label}</div>
        {description && (
          <div className="mt-0.5 text-[11px]" style={{ color: "var(--fill-tertiary)" }}>{description}</div>
        )}
      </div>
      {children}
    </div>
  );
}

function Toggle({ enabled, onChange }: { enabled: boolean; onChange: () => void }) {
  return (
    <button
      onClick={onChange}
      className="relative flex h-[22px] w-[40px] cursor-pointer shrink-0 items-center rounded-full px-0.5 transition-colors duration-200"
      style={{
        background: enabled ? "var(--tint)" : "var(--fill-quaternary)",
        justifyContent: enabled ? "flex-end" : "flex-start",
      }}
    >
      <div className="h-[18px] w-[18px] rounded-full bg-white shadow-sm transition-transform duration-200" />
    </button>
  );
}

/* ━━━ General Tab ━━━ */

function ThemeCard({ preset, selected, resolved, onClick }: {
  preset: typeof ACCENT_PRESETS[number];
  selected: boolean;
  resolved: "light" | "dark";
  onClick: () => void;
}) {
  const p = resolved === "dark" ? preset.preview.dark : preset.preview.light;

  return (
    <button
      onClick={onClick}
      className="group relative flex cursor-pointer flex-col items-center gap-2 outline-none focus-visible:outline-2 focus-visible:outline-offset-4"
      style={{ outlineColor: selected ? p.accent : "var(--tint)" } as React.CSSProperties}
      title={preset.label}
    >
      <div
        className="relative overflow-hidden rounded-[12px] transition-all duration-200 ease-out group-hover:scale-[1.04] group-active:scale-[0.98]"
        style={{
          width: 108,
          height: 72,
          background: p.bg,
          border: selected
            ? `2.5px solid ${p.accent}`
            : "1.5px solid var(--separator-opaque)",
          boxShadow: selected
            ? `0 0 0 2px ${p.accent}30, 0 4px 12px ${p.accent}25`
            : "0 1px 3px rgba(0,0,0,0.08)",
          transform: selected ? "scale(1.03)" : undefined,
        }}
      >
        {/* Sidebar */}
        <div
          className="absolute top-0 left-0 bottom-0"
          style={{ width: 26, background: p.sidebar, borderRight: `0.5px solid ${p.accent}18` }}
        />
        {/* Sidebar dots */}
        {[14, 24, 34].map((top) => (
          <div key={top} className="absolute left-[7px]" style={{ top, width: 12, height: 3, borderRadius: 1.5, background: p.text, opacity: 0.3 }} />
        ))}
        {/* Active sidebar item */}
        <div className="absolute left-[4px]" style={{ top: 8, width: 18, height: 3, borderRadius: 1.5, background: p.accent, opacity: 0.8 }} />

        {/* Header bar */}
        <div className="absolute top-0 left-[26px] right-0" style={{ height: 14, background: p.sidebar, borderBottom: `0.5px solid ${p.accent}18` }} />
        <div className="absolute top-[5px] left-[32px]" style={{ width: 24, height: 3, borderRadius: 1.5, background: p.text, opacity: 0.5 }} />

        {/* Content lines */}
        <div className="absolute top-[20px] left-[32px] right-[8px]" style={{ height: 3, borderRadius: 1.5, background: p.text, opacity: 0.2 }} />
        <div className="absolute top-[27px] left-[32px]" style={{ width: 38, height: 3, borderRadius: 1.5, background: p.text, opacity: 0.15 }} />

        {/* Chat bubble */}
        <div className="absolute right-[6px]" style={{ top: 36, width: 34, height: 10, borderRadius: 5, background: p.accent, opacity: 0.9 }} />
        <div className="absolute left-[32px]" style={{ top: 50, width: 40, height: 10, borderRadius: 5, background: p.sidebar }} />

        {/* Input area */}
        <div className="absolute bottom-[4px] left-[30px] right-[4px]" style={{ height: 10, borderRadius: 4, background: p.sidebar, border: `0.5px solid ${p.accent}25` }} />

        {/* Selected check badge */}
        {selected && (
          <div
            className="absolute top-[3px] right-[3px] flex items-center justify-center rounded-full"
            style={{
              width: 16, height: 16,
              background: p.accent,
              boxShadow: `0 1px 3px ${p.accent}60`,
              animation: "pop 0.2s ease-out",
            }}
          >
            <Check size={9} strokeWidth={3} color="#fff" />
          </div>
        )}
      </div>
      <span
        className="text-[11px] font-medium transition-colors duration-150"
        style={{
          color: selected ? "var(--fill-primary)" : "var(--fill-tertiary)",
          fontWeight: selected ? 600 : 500,
        }}
      >
        {preset.label}
      </span>
    </button>
  );
}

function GeneralTab() {
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

      <div>
        <SectionTitle>行为</SectionTitle>
        <div className="overflow-hidden rounded-[var(--radius-sm)]" style={{ background: "var(--bg-elevated)", border: "0.5px solid var(--separator-opaque)" }}>
          <SettingRow label="桌面通知" description="Agent 完成回复时推送通知">
            <Toggle enabled={notifications} onChange={() => { setNotifications(!notifications); if (loaded) persist("notifications", !notifications); }} />
          </SettingRow>
          <SettingRow label="提示音" description="收到消息时播放提示音">
            <Toggle enabled={sounds} onChange={() => { setSounds(!sounds); if (loaded) persist("sounds", !sounds); }} />
          </SettingRow>
          <SettingRow label="自动滚动" description="新消息时自动滚动到底部" isLast>
            <Toggle enabled={autoScroll} onChange={() => { setAutoScroll(!autoScroll); if (loaded) persist("autoScroll", !autoScroll); }} />
          </SettingRow>
        </div>
      </div>
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

/* ━━━ Models Tab ━━━ */

interface ModelConfigEntry {
  key: string;
  provider: string;
  model: string;
  baseUrl: string;
  temperature: number;
  maxConcurrent: number;
  timeoutSecs: number;
  contextWindow: number;
}

interface CredentialEntry {
  apiKey: string;
  baseUrl: string;
}

const EMPTY_MODEL: Omit<ModelConfigEntry, "key"> = {
  provider: "openai_compatible",
  model: "",
  baseUrl: "",
  temperature: 0,
  maxConcurrent: 10,
  timeoutSecs: 120,
  contextWindow: 0,
};

type TestStatus = "idle" | "testing" | "success" | "error";

function ModelFormModal({
  entry,
  credential,
  isNew,
  onSave,
  onCancel,
  onDelete,
  saving,
}: {
  entry: ModelConfigEntry;
  credential: CredentialEntry;
  isNew: boolean;
  onSave: (e: ModelConfigEntry, c: CredentialEntry) => void;
  onCancel: () => void;
  onDelete?: () => void;
  saving: boolean;
}) {
  const [form, setForm] = useState(entry);
  const [cred, setCred] = useState(credential);
  const [showApiKey, setShowApiKey] = useState(false);
  const [testStatus, setTestStatus] = useState<TestStatus>("idle");
  const [testMsg, setTestMsg] = useState("");
  const [showAdvanced, setShowAdvanced] = useState(false);
  const patch = (k: string, v: string | number) => setForm((f) => ({ ...f, [k]: v }));

  useEffect(() => {
    const handleKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") { e.stopPropagation(); onCancel(); }
    };
    window.addEventListener("keydown", handleKey);
    return () => window.removeEventListener("keydown", handleKey);
  }, [onCancel]);

  const handleTest = async () => {
    const baseUrl = (form.baseUrl || cred.baseUrl || "").replace(/\/+$/, "");
    const apiKey = cred.apiKey;
    if (!baseUrl) { setTestStatus("error"); setTestMsg("请先填写 Base URL"); return; }
    if (!apiKey || apiKey.startsWith("***")) { setTestStatus("error"); setTestMsg("请先填写有效的 API Key"); return; }
    setTestStatus("testing");
    setTestMsg("");
    try {
      if (transport.isTauri) {
        await transport.testModelConnection(baseUrl, apiKey, form.model || undefined);
        setTestStatus("success");
        setTestMsg("连接成功");
      } else {
        const resp = await fetch(`${baseUrl}/models`, {
          method: "GET",
          headers: { Authorization: `Bearer ${apiKey}` },
          signal: AbortSignal.timeout(10000),
        });
        if (resp.ok) {
          setTestStatus("success");
          setTestMsg("连接成功");
        } else {
          const body = await resp.text().catch(() => "");
          setTestStatus("error");
          setTestMsg(`HTTP ${resp.status}${body ? `: ${body.slice(0, 80)}` : ""}`);
        }
      }
    } catch (err: unknown) {
      setTestStatus("error");
      const msg = typeof err === "string" ? err : err instanceof Error ? err.message : "连接失败";
      setTestMsg(msg.length > 120 ? msg.slice(0, 120) + "…" : msg);
    }
  };

  const inputCls = "w-full rounded-[6px] px-3 py-2 text-[13px] outline-none transition-all focus:ring-2 focus:ring-[var(--tint)]";
  const inputStyle: React.CSSProperties = { background: "var(--bg-base)", color: "var(--fill-primary)", border: "0.5px solid var(--separator-opaque)" };
  const labelCls = "mb-1.5 block text-[11px] font-medium";
  const labelStyle: React.CSSProperties = { color: "var(--fill-tertiary)" };

  return (
    <div className="fixed inset-0 z-[60] flex items-center justify-center" onClick={onCancel}>
      <div className="absolute inset-0" style={{ background: "rgba(0,0,0,0.25)" }} />
      <div
        className="relative w-full max-w-[480px] overflow-hidden rounded-[var(--radius-lg)]"
        style={{ background: "var(--bg-elevated)", boxShadow: "var(--shadow-lg)", border: "0.5px solid var(--separator-opaque)", animation: "scale-in 0.15s ease-out" }}
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center justify-between px-5 py-4" style={{ borderBottom: "0.5px solid var(--separator)" }}>
          <h3 className="text-[14px] font-semibold" style={{ color: "var(--fill-primary)" }}>
            {isNew ? "新增模型" : `编辑 · ${entry.key}`}
          </h3>
          <button onClick={onCancel} className="flex h-7 w-7 cursor-pointer items-center justify-center rounded-full transition-colors hover:bg-[var(--bg-hover)]" style={{ color: "var(--fill-tertiary)" }}>
            <X size={14} strokeWidth={1.5} />
          </button>
        </div>
        <div className="max-h-[60vh] space-y-4 overflow-y-auto px-5 py-4">
          <div className="grid grid-cols-2 gap-3">
            <div>
              <label className={labelCls} style={labelStyle}>名称 (key)</label>
              <input value={form.key} onChange={(e) => patch("key", e.target.value)} disabled={!isNew} placeholder="例: dashscope" className={inputCls} style={{ ...inputStyle, opacity: isNew ? 1 : 0.6 }} />
            </div>
            <div>
              <label className={labelCls} style={labelStyle}>Provider</label>
              <div className="relative">
                <select value={form.provider} onChange={(e) => patch("provider", e.target.value)} className={`${inputCls} cursor-pointer pr-8`} style={{ ...inputStyle, appearance: "none" }}>
                  <option value="openai_compatible">OpenAI Compatible</option>
                  <option value="openai">OpenAI</option>
                  <option value="anthropic">Anthropic</option>
                </select>
                <ChevronDown size={10} strokeWidth={2} className="pointer-events-none absolute top-1/2 right-3 -translate-y-1/2" style={{ color: "var(--fill-tertiary)" }} />
              </div>
            </div>
          </div>
          <div className="grid grid-cols-2 gap-3">
            <div>
              <label className={labelCls} style={labelStyle}>模型名称</label>
              <input value={form.model} onChange={(e) => patch("model", e.target.value)} placeholder="例: gpt-4o" className={inputCls} style={inputStyle} />
            </div>
            <div>
              <label className={labelCls} style={labelStyle}>Base URL</label>
              <input value={form.baseUrl} onChange={(e) => patch("baseUrl", e.target.value)} placeholder="https://api.openai.com/v1" className={inputCls} style={inputStyle} />
            </div>
          </div>
          <div>
            <label className={labelCls} style={labelStyle}>
              上下文窗口 (tokens) <span style={{ color: "var(--red, #FC8181)" }}>*</span>
            </label>
            <input
              type="number"
              min="1024"
              step="1024"
              value={form.contextWindow || ""}
              onChange={(e) => patch("contextWindow", parseInt(e.target.value) || 0)}
              placeholder="例: 128000"
              required
              className={inputCls}
              style={{
                ...inputStyle,
                borderColor: form.contextWindow <= 0 ? "var(--red, #FC8181)" : undefined,
              }}
            />
            <p className="mt-1 text-[10px]" style={{ color: form.contextWindow <= 0 ? "var(--red, #FC8181)" : "var(--fill-quaternary)" }}>
              {form.contextWindow <= 0 ? "必填项：请输入模型支持的最大上下文长度" : "模型支持的最大上下文长度，用于自动压缩历史消息"}
            </p>
          </div>
          <div>
            <label className={labelCls} style={labelStyle}>API Key</label>
            <div className="relative">
              <input
                type={showApiKey ? "text" : "password"}
                value={cred.apiKey}
                onChange={(e) => { setCred((c) => ({ ...c, apiKey: e.target.value })); if (testStatus !== "idle") setTestStatus("idle"); }}
                placeholder="sk-..."
                className={`${inputCls} pr-20 font-mono`}
                style={inputStyle}
              />
              <div className="absolute top-1/2 right-2 flex -translate-y-1/2 items-center gap-1">
                <button
                  type="button"
                  onClick={() => setShowApiKey((v) => !v)}
                  className="flex h-7 w-7 cursor-pointer items-center justify-center rounded-[4px] transition-colors hover:bg-[var(--bg-hover)]"
                  title={showApiKey ? "隐藏密钥" : "显示密钥"}
                >
                  {showApiKey
                    ? <EyeOff size={14} strokeWidth={1.5} style={{ color: "var(--fill-tertiary)" }} />
                    : <Eye size={14} strokeWidth={1.5} style={{ color: "var(--fill-tertiary)" }} />
                  }
                </button>
                <button
                  type="button"
                  onClick={handleTest}
                  disabled={testStatus === "testing"}
                  className="flex h-7 cursor-pointer items-center gap-1 rounded-[4px] px-1.5 text-[11px] font-medium transition-colors hover:bg-[var(--bg-hover)] disabled:opacity-50"
                  style={{ color: testStatus === "success" ? "var(--green)" : testStatus === "error" ? "var(--red)" : "var(--tint)" }}
                  title="测试连接"
                >
                  {testStatus === "testing" ? <Loader2 size={13} strokeWidth={1.5} className="animate-spin" />
                    : testStatus === "success" ? <CheckCircle size={13} strokeWidth={1.5} />
                    : testStatus === "error" ? <XCircle size={13} strokeWidth={1.5} />
                    : <Zap size={13} strokeWidth={1.5} />
                  }
                  {testStatus === "idle" && "测试"}
                </button>
              </div>
            </div>
            {testMsg && (
              <p className="mt-1.5 text-[11px]" style={{ color: testStatus === "success" ? "var(--green)" : "var(--red)" }}>
                {testMsg}
              </p>
            )}
          </div>

          <div>
            <button
              type="button"
              onClick={() => setShowAdvanced((v) => !v)}
              className="flex cursor-pointer items-center gap-1.5 text-[11px] font-medium transition-colors hover:opacity-80"
              style={{ color: "var(--fill-tertiary)" }}
            >
              <ChevronDown size={10} strokeWidth={2} style={{ transform: showAdvanced ? "rotate(180deg)" : "rotate(0)", transition: "transform 0.15s" }} />
              高级设置
            </button>
            {showAdvanced && (
              <div className="mt-3 space-y-3">
                {/* Temperature preset selector */}
                <div>
                  <label className={labelCls} style={{ ...labelStyle, display: "flex", alignItems: "center", gap: 4 }}>
                    温度
                    <span style={{ color: "var(--fill-quaternary)", fontWeight: 400 }}>
                      ({form.temperature})
                    </span>
                  </label>
                  {(() => {
                    const TIERS: Array<{ label: string; value: number; desc: string }> = [
                      { label: "精确", value: 0, desc: "确定性强，适合代码和分析" },
                      { label: "均衡", value: 0.7, desc: "通用场景的最佳默认值" },
                      { label: "创意", value: 1.0, desc: "更有创意，适合写作" },
                      { label: "自由", value: 1.5, desc: "高随机性，充满惊喜" },
                    ];
                    const activeIdx = TIERS.findIndex((t) => Math.abs(t.value - form.temperature) < 0.05);
                    return (
                      <div style={{ display: "flex", gap: 4, marginTop: 4 }}>
                        {TIERS.map((tier, i) => {
                          const isActive = i === activeIdx;
                          return (
                            <button
                              key={tier.value}
                              type="button"
                              title={`${tier.desc}（temperature = ${tier.value}）`}
                              onClick={() => patch("temperature", tier.value)}
                              style={{
                                flex: 1,
                                padding: "5px 0",
                                borderRadius: 6,
                                border: `0.5px solid ${isActive ? "var(--tint)" : "var(--separator)"}`,
                                background: isActive ? "var(--tint)" : "var(--bg-secondary)",
                                color: isActive ? "#fff" : "var(--fill-secondary)",
                                fontSize: 11,
                                fontWeight: isActive ? 600 : 400,
                                cursor: "pointer",
                                transition: "all 0.15s",
                                lineHeight: 1.3,
                              }}
                            >
                              <div>{tier.label}</div>
                              <div style={{ fontSize: 9, opacity: 0.75, marginTop: 1 }}>{tier.value}</div>
                            </button>
                          );
                        })}
                      </div>
                    );
                  })()}
                  {/* Custom value input for power users */}
                  <div style={{ marginTop: 5, display: "flex", alignItems: "center", gap: 6 }}>
                    <span style={{ fontSize: 10, color: "var(--fill-quaternary)" }}>自定义：</span>
                    <input
                      type="number"
                      step="0.1"
                      min="0"
                      max="2"
                      value={form.temperature}
                      onChange={(e) => patch("temperature", Math.min(2, Math.max(0, parseFloat(e.target.value) || 0)))}
                      className={inputCls}
                      style={{ ...inputStyle, width: 72, fontSize: 11, padding: "3px 7px" }}
                    />
                  </div>
                </div>
                <div className="grid grid-cols-2 gap-3">
                  <div>
                    <label className={labelCls} style={labelStyle}>并发数</label>
                    <input type="number" min="1" value={form.maxConcurrent} onChange={(e) => patch("maxConcurrent", parseInt(e.target.value) || 1)} className={inputCls} style={inputStyle} />
                  </div>
                  <div>
                    <label className={labelCls} style={labelStyle}>超时 (秒)</label>
                    <input type="number" min="10" value={form.timeoutSecs} onChange={(e) => patch("timeoutSecs", parseInt(e.target.value) || 60)} className={inputCls} style={inputStyle} />
                  </div>
                </div>
              </div>
            )}
          </div>
        </div>

        <div className="flex items-center justify-between px-5 py-3" style={{ borderTop: "0.5px solid var(--separator)", background: "var(--bg-secondary)" }}>
          <div>
            {!isNew && onDelete && (
              <button
                onClick={onDelete}
                disabled={saving}
                className="flex cursor-pointer items-center gap-1 rounded-[6px] px-3 py-1.5 text-[12px] font-medium transition-colors hover:opacity-80"
                style={{ color: "var(--red)" }}
              >
                <Trash2 size={12} strokeWidth={1.5} />
                删除
              </button>
            )}
          </div>
          <div className="flex items-center gap-2">
            <button onClick={onCancel} disabled={saving} className="cursor-pointer rounded-[6px] px-3 py-1.5 text-[12px] font-medium transition-colors hover:bg-[var(--bg-hover)]" style={{ color: "var(--fill-secondary)" }}>
              取消
            </button>
            <button
              onClick={() => onSave(form, cred)}
              disabled={saving || !form.key || !form.model}
              className="rounded-[6px] px-4 py-1.5 text-[12px] font-medium text-white transition-colors hover:opacity-90 disabled:cursor-not-allowed disabled:opacity-50"
              style={{ background: "var(--tint)", cursor: saving || !form.key || !form.model ? "not-allowed" : "pointer" }}
            >
              {saving ? "保存中..." : "保存"}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}

function ModelsTab() {
  const [modelsConfig, setModelsConfig] = useState<Record<string, Record<string, unknown>>>({});
  const [credentials, setCredentials] = useState<Record<string, CredentialEntry>>({});
  const [loading, setLoading] = useState(true);
  const [editing, setEditing] = useState<string | null>(null);
  const [adding, setAdding] = useState(false);
  const [saving, setSaving] = useState(false);
  const [toast, setToast] = useState<{ msg: string; type: "ok" | "err" } | null>(null);

  const showToast = useCallback((msg: string, type: "ok" | "err") => {
    setToast({ msg, type });
    setTimeout(() => setToast(null), 2500);
  }, []);

  const loadData = useCallback(() => {
    setLoading(true);
    Promise.all([
      api.getConfig("models") as Promise<{ key?: string; value?: Record<string, unknown> } | null>,
      api.getConfig("credentials") as Promise<{ key?: string; value?: Record<string, unknown> } | null>,
    ]).then(([modelsCfg, credsCfg]) => {
      const mv = (modelsCfg?.value ?? modelsCfg ?? {}) as Record<string, Record<string, unknown>>;
      setModelsConfig(mv);
      const cv = (credsCfg?.value ?? credsCfg ?? {}) as Record<string, unknown>;
      const mapped: Record<string, CredentialEntry> = {};
      for (const [k, v] of Object.entries(cv)) {
        if (v && typeof v === "object") {
          const obj = v as Record<string, unknown>;
          mapped[k] = { apiKey: (obj.apiKey as string) ?? "", baseUrl: (obj.baseUrl as string) ?? "" };
        }
      }
      setCredentials(mapped);
    }).catch(() => {}).finally(() => setLoading(false));
  }, []);

  useEffect(() => { loadData(); }, [loadData]);

  const entries: ModelConfigEntry[] = Object.entries(modelsConfig)
    .filter(([, v]) => v && typeof v === "object")
    .map(([key, v]) => ({
      key,
      provider: (v.provider as string) ?? "openai_compatible",
      model: (v.model as string) ?? "",
      baseUrl: (v.baseUrl as string) ?? "",
      temperature: (v.temperature as number) ?? 0,
      maxConcurrent: (v.maxConcurrent as number) ?? 10,
      timeoutSecs: (v.timeoutSecs as number) ?? 120,
      contextWindow: (v.contextWindow as number) ?? 0,
    }));

  const handleSave = async (entry: ModelConfigEntry, cred: CredentialEntry) => {
    if (!entry.contextWindow || entry.contextWindow < 1024) {
      alert("请设置上下文窗口大小（至少 1024 tokens）");
      return;
    }
    setSaving(true);
    try {
      const targetKey = entry.key;
      const newModels = { ...modelsConfig };
      if (editing && editing !== entry.key) {
        delete newModels[editing];
      }
      const modelEntry: Record<string, unknown> = {
        provider: entry.provider,
        model: entry.model,
        baseUrl: entry.baseUrl,
        temperature: entry.temperature,
        maxConcurrent: entry.maxConcurrent,
        timeoutSecs: entry.timeoutSecs,
      };
      modelEntry.contextWindow = entry.contextWindow;
      newModels[targetKey] = modelEntry;
      await api.setConfig("models", newModels);

      const existingCred = credentials[targetKey] ?? { apiKey: "", baseUrl: "" };
      const nextCred: CredentialEntry = { ...existingCred };
      const normalizedApiKey = (cred.apiKey ?? "").trim();
      const normalizedBaseUrl = (entry.baseUrl || cred.baseUrl || existingCred.baseUrl || "").trim();
      let credentialChanged = false;

      if (normalizedApiKey && !normalizedApiKey.startsWith("***") && normalizedApiKey !== existingCred.apiKey) {
        nextCred.apiKey = normalizedApiKey;
        credentialChanged = true;
      }
      if (normalizedBaseUrl && normalizedBaseUrl !== existingCred.baseUrl) {
        nextCred.baseUrl = normalizedBaseUrl;
        credentialChanged = true;
      }

      if (credentialChanged) {
        const newCreds = { ...credentials };
        newCreds[targetKey] = nextCred;
        await api.setConfig("credentials", newCreds);
      }

      setEditing(null);
      setAdding(false);
      loadData();
      window.dispatchEvent(new CustomEvent("fastclaw:models-updated"));
      showToast("模型配置已保存", "ok");
    } catch {
      showToast("保存失败", "err");
    } finally {
      setSaving(false);
    }
  };

  const handleDelete = async (key: string) => {
    setSaving(true);
    try {
      const newModels = { ...modelsConfig };
      delete newModels[key];
      await api.setConfig("models", newModels);
      setEditing(null);
      loadData();
      window.dispatchEvent(new CustomEvent("fastclaw:models-updated"));
      showToast(`已删除「${key}」`, "ok");
    } catch {
      showToast("删除失败", "err");
    } finally {
      setSaving(false);
    }
  };

  if (loading) {
    return (
      <div className="flex items-center justify-center py-12">
        <span className="text-[13px]" style={{ color: "var(--fill-tertiary)" }}>加载中...</span>
      </div>
    );
  }

  return (
    <div className="space-y-4">
      {toast && (
        <div
          className="flex items-center gap-2 rounded-[var(--radius-xs)] px-3 py-2 text-[12px] font-medium"
          style={{
            background: toast.type === "ok" ? "color-mix(in srgb, var(--green) 15%, transparent)" : "color-mix(in srgb, var(--red) 15%, transparent)",
            color: toast.type === "ok" ? "var(--green)" : "var(--red)",
            animation: "fade-in 0.15s ease-out",
          }}
        >
          {toast.type === "ok" ? <CheckCircle size={13} strokeWidth={1.5} /> : <XCircle size={13} strokeWidth={1.5} />}
          {toast.msg}
        </div>
      )}
      <div className="flex items-center justify-between">
        <SectionTitle>已配置模型 ({entries.length})</SectionTitle>
        <button
          onClick={() => { setAdding(true); setEditing(null); }}
          className="flex cursor-pointer items-center gap-1 rounded-[6px] px-2.5 py-1 text-[12px] font-medium transition-colors hover:opacity-80"
          style={{ color: "var(--tint)" }}
        >
          <Plus size={12} strokeWidth={2} />
          新增模型
        </button>
      </div>

      <div className="overflow-hidden rounded-[var(--radius-sm)]" style={{ background: "var(--bg-elevated)", border: "0.5px solid var(--separator-opaque)" }}>
        {entries.map((entry, idx) => (
          <div
            key={entry.key}
            className="group cursor-pointer px-4 py-3 transition-colors duration-100 hover:bg-[var(--bg-hover)]"
            style={idx < entries.length - 1 ? { borderBottom: "0.5px solid var(--separator)" } : undefined}
            onClick={() => { setEditing(entry.key); setAdding(false); }}
          >
            <div className="flex items-center justify-between gap-2">
              <div className="min-w-0 flex-1">
                <div className="flex min-w-0 items-center gap-2">
                  <span className="min-w-0 truncate text-[13px] font-semibold" style={{ color: "var(--fill-primary)" }} title={entry.key}>
                    {entry.key}
                  </span>
                  <span className="shrink-0 rounded-full px-1.5 py-0.5 text-[10px] font-medium" style={{ background: "var(--bg-tertiary)", color: "var(--fill-secondary)" }}>
                    {entry.model}
                  </span>
                </div>
                <div className="mt-1 flex items-center gap-3 text-[11px]" style={{ color: "var(--fill-tertiary)" }}>
                  <span>{entry.provider}</span>
                  {entry.baseUrl && <><span>·</span><span className="max-w-[200px] truncate">{entry.baseUrl}</span></>}
                </div>
                {credentials[entry.key]?.apiKey && (
                  <div className="mt-1 flex items-center gap-1.5 text-[11px]">
                    <span className="inline-block h-[6px] w-[6px] rounded-full" style={{ background: "var(--green)" }} />
                    <span style={{ color: "var(--fill-tertiary)" }}>已配置密钥</span>
                  </div>
                )}
              </div>
              <Pencil size={14} strokeWidth={1.5} className="shrink-0 opacity-0 transition-opacity group-hover:opacity-100" style={{ color: "var(--fill-quaternary)" }} />
            </div>
          </div>
        ))}
      </div>

      {entries.length === 0 && (
        <div className="py-8 text-center">
          <p className="text-[13px]" style={{ color: "var(--fill-tertiary)" }}>
            暂无已配置模型，点击上方"新增模型"添加
          </p>
        </div>
      )}

      <p className="text-[11px]" style={{ color: "var(--fill-quaternary)" }}>
        点击模型卡片即可编辑，修改会持久化到 ~/.fastclaw/config/default.json（部分配置重启后生效）
      </p>

      {(editing || adding) && (
        <ModelFormModal
          entry={editing ? entries.find((e) => e.key === editing)! : { key: "", ...EMPTY_MODEL }}
          credential={editing ? (credentials[editing] ?? { apiKey: "", baseUrl: "" }) : { apiKey: "", baseUrl: "" }}
          isNew={adding}
          onSave={handleSave}
          onCancel={() => { setEditing(null); setAdding(false); }}
          onDelete={editing ? () => handleDelete(editing) : undefined}
          saving={saving}
        />
      )}
    </div>
  );
}

/* ━━━ Web Search Tab ━━━ */

type WebSearchBackend = "tavily" | "searxng" | "builtin" | "";

const BUILTIN_ENGINES = [
  { id: "google", label: "Google" },
  { id: "baidu", label: "百度 (Baidu)" },
  { id: "bing", label: "Bing" },
  { id: "sogou", label: "搜狗 (Sogou)" },
  { id: "360", label: "360搜索 (360 Search)" },
] as const;

function WebSearchTab() {
  const [backend, setBackend] = useState<WebSearchBackend>("");
  const [tavilyKey, setTavilyKey] = useState("");
  const [searxngUrl, setSearxngUrl] = useState("");
  const [enabledEngines, setEnabledEngines] = useState<Set<string>>(new Set(BUILTIN_ENGINES.map(e => e.id)));
  const [showKey, setShowKey] = useState(false);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [toast, setToast] = useState<{ msg: string; type: "ok" | "err" } | null>(null);
  const [testStatus, setTestStatus] = useState<TestStatus>("idle");
  const [testMsg, setTestMsg] = useState("");

  const showToast = useCallback((msg: string, type: "ok" | "err") => {
    setToast({ msg, type });
    setTimeout(() => setToast(null), 2500);
  }, []);

  useEffect(() => {
    Promise.all([
      api.getConfig("webSearch") as Promise<{ key?: string; value?: Record<string, unknown> } | null>,
      api.getConfig("credentials") as Promise<{ key?: string; value?: Record<string, unknown> } | null>,
    ]).then(([wsCfg, credsCfg]) => {
      const ws = (wsCfg?.value ?? wsCfg ?? {}) as Record<string, unknown>;
      const be = (ws.backend as string) ?? "";
      setBackend((be === "tavily" || be === "searxng" || be === "builtin") ? be : "");
      setSearxngUrl((ws.baseUrl as string) ?? "");
      if (Array.isArray(ws.engines) && ws.engines.length > 0) {
        setEnabledEngines(new Set(ws.engines as string[]));
      }

      const creds = (credsCfg?.value ?? credsCfg ?? {}) as Record<string, unknown>;
      const tavilyCred = creds.tavily as Record<string, unknown> | undefined;
      if (tavilyCred?.apiKey) {
        const key = tavilyCred.apiKey as string;
        setTavilyKey(key.length > 8 ? `${key.slice(0, 4)}${"*".repeat(key.length - 8)}${key.slice(-4)}` : "***");
      }
    }).catch(() => {}).finally(() => setLoading(false));
  }, []);

  const handleSave = useCallback(async () => {
    setSaving(true);
    try {
      const wsConfig: Record<string, unknown> = { backend };
      if (backend === "searxng") {
        wsConfig.baseUrl = searxngUrl.trim() || null;
      }
      if (backend === "tavily" && tavilyKey && !tavilyKey.includes("*")) {
        wsConfig.apiKey = tavilyKey.trim();
      }
      if (backend === "builtin") {
        wsConfig.engines = [...enabledEngines];
      }
      await api.setConfig("webSearch", wsConfig);

      if (backend === "tavily" && tavilyKey && !tavilyKey.includes("*")) {
        const credsCfg = await api.getConfig("credentials") as { key?: string; value?: Record<string, unknown> } | null;
        const creds = (credsCfg?.value ?? credsCfg ?? {}) as Record<string, unknown>;
        const existing = (creds.tavily ?? {}) as Record<string, unknown>;
        creds.tavily = { ...existing, apiKey: tavilyKey.trim() };
        await api.setConfig("credentials", creds);
      }

      showToast("搜索配置已保存（重启后生效）", "ok");
    } catch {
      showToast("保存失败", "err");
    } finally {
      setSaving(false);
    }
  }, [backend, tavilyKey, searxngUrl, enabledEngines, showToast]);

  const handleTest = useCallback(async () => {
    setTestStatus("testing");
    setTestMsg("");
    try {
      if (backend === "tavily") {
        const key = tavilyKey.includes("*") ? "" : tavilyKey.trim();
        if (!key) {
          setTestStatus("error");
          setTestMsg("请先填写有效的 API Key");
          return;
        }
        const resp = await fetch("https://api.tavily.com/search", {
          method: "POST",
          headers: { "Content-Type": "application/json", Authorization: `Bearer ${key}` },
          body: JSON.stringify({ query: "test", max_results: 1, search_depth: "basic" }),
          signal: AbortSignal.timeout(10000),
        });
        if (resp.ok) {
          setTestStatus("success");
          setTestMsg("Tavily 连接成功");
        } else {
          const body = await resp.text().catch(() => "");
          setTestStatus("error");
          setTestMsg(`HTTP ${resp.status}${body ? `: ${body.slice(0, 80)}` : ""}`);
        }
      } else if (backend === "searxng") {
        const base = searxngUrl.trim().replace(/\/+$/, "");
        if (!base) {
          setTestStatus("error");
          setTestMsg("请先填写 SearXNG 实例 URL");
          return;
        }
        const resp = await fetch(`${base}/search?q=test&format=json&categories=general`, {
          signal: AbortSignal.timeout(10000),
        });
        if (resp.ok) {
          setTestStatus("success");
          setTestMsg("SearXNG 连接成功");
        } else {
          setTestStatus("error");
          setTestMsg(`HTTP ${resp.status}`);
        }
      } else {
        setTestStatus("error");
        setTestMsg("请先选择搜索引擎");
      }
    } catch (err) {
      setTestStatus("error");
      const msg = typeof err === "string" ? err : err instanceof Error ? err.message : "连接失败";
      setTestMsg(msg.length > 120 ? msg.slice(0, 120) + "…" : msg);
    }
  }, [backend, tavilyKey, searxngUrl]);

  if (loading) {
    return (
      <div className="flex items-center justify-center py-12">
        <span className="text-[13px]" style={{ color: "var(--fill-tertiary)" }}>加载中...</span>
      </div>
    );
  }

  const inputCls = "w-full rounded-[6px] px-3 py-2 text-[13px] outline-none transition-all focus:ring-2 focus:ring-[var(--tint)]";
  const inputStyle: React.CSSProperties = { background: "var(--bg-base)", color: "var(--fill-primary)", border: "0.5px solid var(--separator-opaque)" };
  const labelCls = "mb-1.5 block text-[11px] font-medium";
  const labelStyle: React.CSSProperties = { color: "var(--fill-tertiary)" };

  return (
    <div className="space-y-6">
      {toast && (
        <div
          className="flex items-center gap-2 rounded-[var(--radius-xs)] px-3 py-2 text-[12px] font-medium"
          style={{
            background: toast.type === "ok" ? "color-mix(in srgb, var(--green) 15%, transparent)" : "color-mix(in srgb, var(--red) 15%, transparent)",
            color: toast.type === "ok" ? "var(--green)" : "var(--red)",
            animation: "fade-in 0.15s ease-out",
          }}
        >
          {toast.type === "ok" ? <CheckCircle size={13} strokeWidth={1.5} /> : <XCircle size={13} strokeWidth={1.5} />}
          {toast.msg}
        </div>
      )}

      <div>
        <SectionTitle>搜索引擎</SectionTitle>
        <p className="mb-3 text-[12px]" style={{ color: "var(--fill-tertiary)" }}>
          配置 Agent 使用的 web_search 工具后端，用于联网搜索实时信息
        </p>
        <div className="overflow-hidden rounded-[var(--radius-sm)]" style={{ background: "var(--bg-elevated)", border: "0.5px solid var(--separator-opaque)" }}>
          {([
            { value: "builtin" as const, label: "内置搜索", desc: "直接抓取 Google、百度、Bing 等公开搜索页面，无需 API Key" },
            { value: "tavily" as const, label: "Tavily", desc: "商业 API，搜索质量高，需要 API Key" },
            { value: "searxng" as const, label: "SearXNG", desc: "开源自托管元搜索引擎，无需 API Key" },
          ]).map((opt, idx, arr) => (
            <div
              key={opt.value}
              className="flex cursor-pointer items-center gap-3 px-4 py-3 transition-colors duration-100 hover:bg-[var(--bg-hover)]"
              style={idx < arr.length - 1 ? { borderBottom: "0.5px solid var(--separator)" } : undefined}
              onClick={() => setBackend(opt.value)}
            >
              <div
                className="flex h-[18px] w-[18px] shrink-0 items-center justify-center rounded-full"
                style={{
                  border: `2px solid ${backend === opt.value ? "var(--tint)" : "var(--fill-quaternary)"}`,
                  background: backend === opt.value ? "var(--tint)" : "transparent",
                  transition: "all 0.15s",
                }}
              >
                {backend === opt.value && <div className="h-[6px] w-[6px] rounded-full bg-white" />}
              </div>
              <div className="min-w-0 flex-1">
                <div className="text-[13px] font-semibold" style={{ color: "var(--fill-primary)" }}>{opt.label}</div>
                <div className="mt-0.5 text-[11px]" style={{ color: "var(--fill-tertiary)" }}>{opt.desc}</div>
              </div>
            </div>
          ))}
        </div>
      </div>

      {backend === "builtin" && (
        <div>
          <SectionTitle>搜索引擎选择</SectionTitle>
          <p className="mb-3 text-[11px]" style={{ color: "var(--fill-tertiary)" }}>
            选择要启用的搜索引擎，搜索时将并行查询所有已启用的引擎并合并结果
          </p>
          <div className="overflow-hidden rounded-[var(--radius-sm)]" style={{ background: "var(--bg-elevated)", border: "0.5px solid var(--separator-opaque)" }}>
            {BUILTIN_ENGINES.map((eng, idx) => {
              const checked = enabledEngines.has(eng.id);
              return (
                <div
                  key={eng.id}
                  className="flex cursor-pointer items-center gap-3 px-4 py-2.5 transition-colors duration-100 hover:bg-[var(--bg-hover)]"
                  style={idx < BUILTIN_ENGINES.length - 1 ? { borderBottom: "0.5px solid var(--separator)" } : undefined}
                  onClick={() => {
                    setEnabledEngines(prev => {
                      const next = new Set(prev);
                      if (next.has(eng.id)) {
                        if (next.size > 1) next.delete(eng.id);
                      } else {
                        next.add(eng.id);
                      }
                      return next;
                    });
                  }}
                >
                  <div
                    className="flex h-[16px] w-[16px] shrink-0 items-center justify-center rounded-[3px]"
                    style={{
                      border: `1.5px solid ${checked ? "var(--tint)" : "var(--fill-quaternary)"}`,
                      background: checked ? "var(--tint)" : "transparent",
                      transition: "all 0.15s",
                    }}
                  >
                    {checked && (
                      <svg width="10" height="10" viewBox="0 0 10 10" fill="none" xmlns="http://www.w3.org/2000/svg">
                        <path d="M2 5L4.2 7.5L8 2.5" stroke="white" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" />
                      </svg>
                    )}
                  </div>
                  <span className="text-[13px]" style={{ color: "var(--fill-primary)" }}>{eng.label}</span>
                </div>
              );
            })}
          </div>
          <p className="mt-2 text-[11px]" style={{ color: "var(--fill-quaternary)" }}>
            至少需要启用一个搜索引擎 · 已选中 {enabledEngines.size}/{BUILTIN_ENGINES.length}
          </p>
        </div>
      )}

      {backend === "tavily" && (
        <div>
          <SectionTitle>Tavily 配置</SectionTitle>
          <div className="space-y-3">
            <div>
              <label className={labelCls} style={labelStyle}>API Key</label>
              <div className="relative">
                <input
                  type={showKey ? "text" : "password"}
                  value={tavilyKey}
                  onChange={(e) => { setTavilyKey(e.target.value); if (testStatus !== "idle") setTestStatus("idle"); }}
                  placeholder="tvly-..."
                  className={`${inputCls} pr-20 font-mono`}
                  style={inputStyle}
                />
                <div className="absolute top-1/2 right-2 flex -translate-y-1/2 items-center gap-1">
                  <button
                    type="button"
                    onClick={() => setShowKey((v) => !v)}
                    className="flex h-7 w-7 cursor-pointer items-center justify-center rounded-[4px] transition-colors hover:bg-[var(--bg-hover)]"
                  >
                    {showKey
                      ? <EyeOff size={14} strokeWidth={1.5} style={{ color: "var(--fill-tertiary)" }} />
                      : <Eye size={14} strokeWidth={1.5} style={{ color: "var(--fill-tertiary)" }} />
                    }
                  </button>
                  <button
                    type="button"
                    onClick={handleTest}
                    disabled={testStatus === "testing"}
                    className="flex h-7 cursor-pointer items-center gap-1 rounded-[4px] px-1.5 text-[11px] font-medium transition-colors hover:bg-[var(--bg-hover)] disabled:opacity-50"
                    style={{ color: testStatus === "success" ? "var(--green)" : testStatus === "error" ? "var(--red)" : "var(--tint)" }}
                  >
                    {testStatus === "testing" ? <Loader2 size={13} strokeWidth={1.5} className="animate-spin" />
                      : testStatus === "success" ? <CheckCircle size={13} strokeWidth={1.5} />
                      : testStatus === "error" ? <XCircle size={13} strokeWidth={1.5} />
                      : <Zap size={13} strokeWidth={1.5} />
                    }
                    {testStatus === "idle" && "测试"}
                  </button>
                </div>
              </div>
              {testMsg && (
                <p className="mt-1.5 text-[11px]" style={{ color: testStatus === "success" ? "var(--green)" : "var(--red)" }}>
                  {testMsg}
                </p>
              )}
            </div>
            <p className="text-[11px]" style={{ color: "var(--fill-quaternary)" }}>
              前往 <a href="https://tavily.com" target="_blank" rel="noreferrer" className="underline" style={{ color: "var(--tint)" }}>tavily.com</a> 注册获取 API Key
            </p>
          </div>
        </div>
      )}

      {backend === "searxng" && (
        <div>
          <SectionTitle>SearXNG 配置</SectionTitle>
          <div className="space-y-3">
            <div>
              <label className={labelCls} style={labelStyle}>实例 URL</label>
              <div className="relative">
                <input
                  value={searxngUrl}
                  onChange={(e) => { setSearxngUrl(e.target.value); if (testStatus !== "idle") setTestStatus("idle"); }}
                  placeholder="http://localhost:8888"
                  className={`${inputCls} pr-16 font-mono`}
                  style={inputStyle}
                />
                <button
                  type="button"
                  onClick={handleTest}
                  disabled={testStatus === "testing"}
                  className="absolute top-1/2 right-2 flex h-7 -translate-y-1/2 cursor-pointer items-center gap-1 rounded-[4px] px-1.5 text-[11px] font-medium transition-colors hover:bg-[var(--bg-hover)] disabled:opacity-50"
                  style={{ color: testStatus === "success" ? "var(--green)" : testStatus === "error" ? "var(--red)" : "var(--tint)" }}
                >
                  {testStatus === "testing" ? <Loader2 size={13} strokeWidth={1.5} className="animate-spin" />
                    : testStatus === "success" ? <CheckCircle size={13} strokeWidth={1.5} />
                    : testStatus === "error" ? <XCircle size={13} strokeWidth={1.5} />
                    : <Zap size={13} strokeWidth={1.5} />
                  }
                  {testStatus === "idle" && "测试"}
                </button>
              </div>
              {testMsg && (
                <p className="mt-1.5 text-[11px]" style={{ color: testStatus === "success" ? "var(--green)" : "var(--red)" }}>
                  {testMsg}
                </p>
              )}
            </div>
            <p className="text-[11px]" style={{ color: "var(--fill-quaternary)" }}>
              确保实例已启用 JSON 输出格式（/search?format=json）
            </p>
          </div>
        </div>
      )}

      {!backend && (
        <div className="rounded-[var(--radius-sm)] px-4 py-6 text-center" style={{ background: "var(--bg-elevated)", border: "0.5px solid var(--separator-opaque)" }}>
          <Search size={24} strokeWidth={1.5} className="mx-auto mb-2" style={{ color: "var(--fill-quaternary)" }} />
          <p className="text-[13px]" style={{ color: "var(--fill-tertiary)" }}>
            选择一个搜索引擎后端以启用 Agent 联网搜索能力
          </p>
        </div>
      )}

      <div className="flex items-center justify-end gap-2">
        <button
          onClick={handleSave}
          disabled={saving}
          className="rounded-[6px] px-4 py-2 text-[13px] font-medium text-white transition-colors hover:opacity-90 disabled:opacity-50"
          style={{ background: "var(--tint)", cursor: saving ? "not-allowed" : "pointer" }}
        >
          {saving ? "保存中..." : "保存"}
        </button>
      </div>

      <p className="text-[11px]" style={{ color: "var(--fill-quaternary)" }}>
        搜索配置保存到 ~/.fastclaw/config/default.json，修改后需重启应用生效
      </p>
    </div>
  );
}

/* ━━━ Skills Tab ━━━ */

function SkillsTab() {
  const [publicSkills, setPublicSkills] = useState<api.SkillInfo[]>([]);
  const [agentSkillsMap, setAgentSkillsMap] = useState<Record<string, api.SkillInfo[]>>({});
  const [tools, setTools] = useState<api.ToolInfo[]>([]);
  const [loading, setLoading] = useState(true);
  const [filter, setFilter] = useState<"skills" | "tools">("skills");
  const [refreshing, setRefreshing] = useState(false);
  const [uploading, setUploading] = useState(false);
  const [skillMenuOpen, setSkillMenuOpen] = useState(false);
  const gatewayReady = useGatewayStore((s) => s.connected);

  const loadAllSkills = useCallback(async () => {
    try {
      const [globalSkills, agentsResp] = await Promise.all([
        api.listSkills(),
        api.getConfig("agents") as Promise<{ key?: string; value?: { list?: { id: string; name: string }[] } } | null>,
      ]);
      setPublicSkills(globalSkills);
      const agentList = ((agentsResp?.value ?? agentsResp) as { list?: { id: string; name: string }[] } | null)?.list ?? [];
      const agentMap: Record<string, api.SkillInfo[]> = {};
      const results = await Promise.allSettled(
        agentList.map(async (a) => {
          const skills = await api.listSkills(a.id);
          return { id: a.id, skills };
        })
      );
      for (const r of results) {
        if (r.status === "fulfilled") {
          agentMap[r.value.id] = r.value.skills;
        }
      }
      setAgentSkillsMap(agentMap);
    } catch { /* silent */ }
  }, []);

  useEffect(() => {
    if (!gatewayReady) return;
    const loadAll = async () => {
      const [, toolList] = await Promise.all([
        loadAllSkills(),
        api.listTools(),
      ]);
      if (toolList) setTools(toolList);
      setLoading(false);
    };
    loadAll();
  }, [gatewayReady, loadAllSkills]);

  const handleRefresh = useCallback(async () => {
    setRefreshing(true);
    await api.refreshSkills();
    await loadAllSkills();
    setRefreshing(false);
  }, [loadAllSkills]);

  const handleUploadFolder = useCallback(async () => {
    setUploading(true);
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const selected = await open({ title: "选择 Skill 文件夹（需包含 SKILL.md）", directory: true, multiple: false });
      if (selected) {
        await api.uploadSkill(selected as string);
        await api.refreshSkills();
        await loadAllSkills();
      }
    } catch { /* cancelled */ }
    setUploading(false);
  }, [loadAllSkills]);

  const handleUploadZip = useCallback(async () => {
    setUploading(true);
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const selected = await open({ title: "选择 Skill ZIP 文件", directory: false, multiple: false, filters: [{ name: "ZIP", extensions: ["zip"] }] });
      if (selected) {
        await api.uploadSkill(selected as string);
        await api.refreshSkills();
        await loadAllSkills();
      }
    } catch { /* cancelled */ }
    setUploading(false);
  }, [loadAllSkills]);

  const totalSkills = useMemo(
    () => publicSkills.length + Object.values(agentSkillsMap).reduce((s, a) => s + a.length, 0),
    [publicSkills, agentSkillsMap],
  );

  if (loading) {
    return (
      <div className="flex items-center justify-center py-12">
        <span className="text-[13px]" style={{ color: "var(--fill-tertiary)" }}>加载中...</span>
      </div>
    );
  }

  const SkillRow = ({ skill, isLast }: { skill: api.SkillInfo; isLast: boolean }) => (
    <div
      className="px-4 py-3 transition-colors duration-100 hover:bg-[var(--bg-hover)]"
      style={!isLast ? { borderBottom: "0.5px solid var(--separator)" } : undefined}
    >
      <div className="flex items-center justify-between">
        <div className="min-w-0 flex-1">
          <div className="flex min-w-0 items-center gap-2">
            <span className="min-w-0 truncate text-[13px] font-semibold" style={{ color: "var(--fill-primary)" }} title={skill.name}>{skill.name}</span>
            {skill.version && <span className="shrink-0 text-[10px]" style={{ color: "var(--fill-quaternary)" }}>v{skill.version}</span>}
          </div>
          <div className="mt-0.5 line-clamp-2 text-[11px]" style={{ color: "var(--fill-tertiary)" }}>{skill.description}</div>
          {skill.tags && skill.tags.length > 0 && (
            <div className="mt-1.5 flex flex-wrap gap-1">
              {skill.tags.map((tag) => (
                <span key={tag} className="rounded-full px-1.5 py-0.5 text-[10px]" style={{ background: "var(--bg-tertiary)", color: "var(--fill-tertiary)" }}>
                  {tag}
                </span>
              ))}
            </div>
          )}
        </div>
      </div>
    </div>
  );

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <SectionTitle>能力管理</SectionTitle>
        <div className="flex items-center gap-2">
          {filter === "skills" && (
            <div className="flex items-center gap-1">
              <button
                onClick={handleRefresh}
                disabled={refreshing}
                className="cursor-pointer rounded-[var(--radius-xs)] p-1.5 transition-colors duration-100 hover:bg-[var(--bg-hover)] disabled:opacity-40"
                title="刷新 Skills"
              >
                <RefreshCw size={13} strokeWidth={1.5} className={refreshing ? "animate-spin" : ""} style={{ color: "var(--fill-tertiary)" }} />
              </button>
              <div className="relative">
                <button
                  onClick={() => setSkillMenuOpen((v) => !v)}
                  disabled={uploading}
                  className="cursor-pointer rounded-[var(--radius-xs)] p-1.5 transition-colors duration-100 hover:bg-[var(--bg-hover)] disabled:opacity-40"
                  title="上传 Skill"
                >
                  <Upload size={13} strokeWidth={1.5} style={{ color: "var(--fill-tertiary)" }} />
                </button>
                {skillMenuOpen && (
                  <div
                    className="absolute right-0 top-full z-50 mt-1 min-w-[140px] overflow-hidden rounded-[var(--radius-sm)] py-1 shadow-lg"
                    style={{ background: "var(--bg-elevated)", border: "0.5px solid var(--separator-opaque)" }}
                    onMouseLeave={() => setSkillMenuOpen(false)}
                  >
                    <button
                      onClick={() => { setSkillMenuOpen(false); handleUploadFolder(); }}
                      className="w-full cursor-pointer px-3 py-2 text-left text-[12px] transition-colors hover:bg-[var(--bg-hover)]"
                      style={{ color: "var(--fill-primary)" }}
                    >
                      <FolderOpen size={12} className="mr-2 inline" strokeWidth={1.5} />选择文件夹
                    </button>
                    <button
                      onClick={() => { setSkillMenuOpen(false); handleUploadZip(); }}
                      className="w-full cursor-pointer px-3 py-2 text-left text-[12px] transition-colors hover:bg-[var(--bg-hover)]"
                      style={{ color: "var(--fill-primary)" }}
                    >
                      <FileText size={12} className="mr-2 inline" strokeWidth={1.5} />选择 ZIP 文件
                    </button>
                  </div>
                )}
              </div>
            </div>
          )}
          <div className="flex rounded-[6px] p-0.5" style={{ background: "var(--bg-tertiary)" }}>
            {(["skills", "tools"] as const).map((f) => (
              <button
                key={f}
                onClick={() => setFilter(f)}
                className="rounded-[3px] px-2.5 py-1 text-[11px] font-medium transition-all duration-150"
                style={{
                  background: filter === f ? "var(--bg-elevated)" : "transparent",
                  color: filter === f ? "var(--fill-primary)" : "var(--fill-tertiary)",
                  boxShadow: filter === f ? "var(--shadow-sm)" : "none",
                }}
              >
                {f === "skills" ? `Skills (${totalSkills})` : `Tools (${tools.length})`}
              </button>
            ))}
          </div>
        </div>
      </div>

      <div className="space-y-3">
        {filter === "skills" ? (
          <>
            {/* Public / Global skills */}
            <div>
              <div className="mb-2 flex items-center gap-2 text-[11px] font-medium" style={{ color: "var(--fill-tertiary)" }}>
                <Globe size={12} strokeWidth={1.5} />
                公共 Skills ({publicSkills.length})
              </div>
              {publicSkills.length === 0 ? (
                <p className="rounded-[var(--radius-sm)] px-4 py-3 text-center text-[12px]" style={{ background: "var(--bg-elevated)", border: "0.5px solid var(--separator-opaque)", color: "var(--fill-tertiary)" }}>
                  暂无公共 Skill
                </p>
              ) : (
                <div className="overflow-hidden rounded-[var(--radius-sm)]" style={{ background: "var(--bg-elevated)", border: "0.5px solid var(--separator-opaque)" }}>
                  {publicSkills.map((skill, idx) => <SkillRow key={skill.id} skill={skill} isLast={idx === publicSkills.length - 1} />)}
                </div>
              )}
            </div>
            {/* Per-agent skills */}
            {Object.entries(agentSkillsMap).map(([agentId, skills]) => (
              skills.length > 0 && (
                <div key={agentId}>
                  <div className="mb-2 flex items-center gap-2 text-[11px] font-medium" style={{ color: "var(--fill-tertiary)" }}>
                    <User size={12} strokeWidth={1.5} />
                    Agent: {agentId} ({skills.length})
                  </div>
                  <div className="overflow-hidden rounded-[var(--radius-sm)]" style={{ background: "var(--bg-elevated)", border: "0.5px solid var(--separator-opaque)" }}>
                    {skills.map((skill, idx) => <SkillRow key={`${agentId}-${skill.id}`} skill={skill} isLast={idx === skills.length - 1} />)}
                  </div>
                </div>
              )
            ))}
          </>
        ) : (
          tools.length === 0 ? (
            <p className="py-4 text-center text-[13px]" style={{ color: "var(--fill-tertiary)" }}>暂无已注册 Tool</p>
          ) : (
            <div className="overflow-hidden rounded-[var(--radius-sm)]" style={{ background: "var(--bg-elevated)", border: "0.5px solid var(--separator-opaque)" }}>
              {tools.map((tool, idx) => (
                <div
                  key={tool.id}
                  className="flex items-center justify-between px-4 py-3 transition-colors duration-100 hover:bg-[var(--bg-hover)]"
                  style={idx < tools.length - 1 ? { borderBottom: "0.5px solid var(--separator)" } : undefined}
                >
                  <div className="min-w-0 flex-1">
                    <div className="text-[13px] font-semibold" style={{ color: "var(--fill-primary)" }}>{tool.name}</div>
                    {tool.description && <div className="mt-0.5 text-[11px]" style={{ color: "var(--fill-tertiary)" }}>{tool.description}</div>}
                  </div>
                  <span className="text-[10px] font-mono" style={{ color: "var(--fill-quaternary)" }}>{tool.id}</span>
                </div>
              ))}
            </div>
          )
        )}
      </div>
    </div>
  );
}

/* ━━━ Gateway Tab ━━━ */

function GatewayTab() {
  const gwInfo = useGatewayStore((s) => s.info);
  const gwMode = useGatewayStore((s) => s.mode);
  const connected = useGatewayStore((s) => s.connected);

  const [gwConfig, setGwConfig] = useState<{ port?: number; host?: string } | null>(null);

  useEffect(() => {
    api.getConfig("gateway").then((data) => {
      const cfg = data as { key?: string; value?: { port?: number; host?: string } } | null;
      setGwConfig((cfg?.value ?? cfg) as { port?: number; host?: string } | null);
    }).catch(() => {});
  }, []);

  const modeLabel = gwMode === "embedded" ? "内嵌网关" : gwMode === "remote" ? "远程网关" : gwMode === "browser" ? "浏览器开发" : "连接中...";

  return (
    <div className="space-y-6">
      <div className="space-y-2">
        <SectionTitle>运行状态</SectionTitle>
        {(() => {
          const rows = [
            { label: "模式", value: modeLabel },
            { label: "状态", value: connected ? "已连接" : "未连接", dot: connected },
            ...(gwInfo ? [
              { label: "端口", value: String(gwInfo.port) },
              { label: "版本", value: gwInfo.version },
              { label: "WebSocket", value: gwInfo.wsUrl },
              { label: "HTTP", value: gwInfo.httpUrl },
            ] : []),
          ];
          return (
            <div className="overflow-hidden rounded-[var(--radius-sm)]" style={{ background: "var(--bg-elevated)", border: "0.5px solid var(--separator-opaque)" }}>
              {rows.map(({ label, value, dot }, idx) => (
                <div key={label} className="flex items-center justify-between gap-3 px-4 py-2.5" style={idx < rows.length - 1 ? { borderBottom: "0.5px solid var(--separator)" } : undefined}>
                  <span className="shrink-0 text-[13px]" style={{ color: "var(--fill-secondary)" }}>{label}</span>
                  <div className="flex min-w-0 items-center gap-1.5">
                    {dot !== undefined && (
                      <span className="inline-block h-[6px] w-[6px] shrink-0 rounded-full" style={{ background: dot ? "var(--green)" : "var(--red)" }} />
                    )}
                    <span className="min-w-0 truncate text-[13px] font-medium font-mono" style={{ color: "var(--fill-primary)" }} title={value}>{value}</span>
                  </div>
                </div>
              ))}
            </div>
          );
        })()}
      </div>
      {gwConfig && (
        <div>
          <SectionTitle>配置</SectionTitle>
          <div className="overflow-hidden rounded-[var(--radius-sm)]" style={{ background: "var(--bg-elevated)", border: "0.5px solid var(--separator-opaque)" }}>
            <div className="px-4 py-2.5" style={gwConfig.host ? { borderBottom: "0.5px solid var(--separator)" } : undefined}>
              <span className="text-[11px]" style={{ color: "var(--fill-tertiary)" }}>配置端口</span>
              <div className="text-[13px] font-mono" style={{ color: "var(--fill-primary)" }}>{gwConfig.port ?? "默认"}</div>
            </div>
            {gwConfig.host && (
              <div className="px-4 py-2.5">
                <span className="text-[11px]" style={{ color: "var(--fill-tertiary)" }}>绑定地址</span>
                <div className="text-[13px] font-mono" style={{ color: "var(--fill-primary)" }}>{gwConfig.host}</div>
              </div>
            )}
          </div>
          <p className="mt-2 text-[11px]" style={{ color: "var(--fill-quaternary)" }}>
            修改网关配置需编辑 ~/.fastclaw/config/default.json 并重启
          </p>
        </div>
      )}
    </div>
  );
}

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

function McpTab() {
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
        <input style={inputStyle} value={draft.id} disabled={!!editingId} onChange={(e) => setDraft({ ...draft, id: e.target.value })} placeholder="e.g. chrome-devtools" />
      </div>
      <div>
        <label className="mb-1 block text-[11px] font-medium" style={{ color: "var(--fill-tertiary)" }}>Command</label>
        <input style={inputStyle} value={draft.command} onChange={(e) => setDraft({ ...draft, command: e.target.value })} placeholder="e.g. npx" />
      </div>
      <div>
        <label className="mb-1 block text-[11px] font-medium" style={{ color: "var(--fill-tertiary)" }}>Args (逗号分隔)</label>
        <input style={inputStyle} value={draft.args.join(", ")} onChange={(e) => setDraft({ ...draft, args: e.target.value.split(",").map((a) => a.trim()).filter(Boolean) })} placeholder="e.g. -y, @anthropic-ai/chrome-devtools-mcp@latest" />
      </div>
      <div className="flex gap-2 pt-1">
        <button onClick={saveDraft} disabled={!draft.id.trim() || !draft.command.trim()} className="rounded-[var(--radius-xs)] px-3 py-1.5 text-[12px] font-medium transition-colors duration-100" style={{ background: "var(--accent)", color: "#fff", opacity: (!draft.id.trim() || !draft.command.trim()) ? 0.5 : 1, cursor: (!draft.id.trim() || !draft.command.trim()) ? "not-allowed" : "pointer" }}>
          {editingId ? "保存" : "添加"}
        </button>
        <button onClick={cancelEdit} className="rounded-[var(--radius-xs)] px-3 py-1.5 text-[12px] font-medium cursor-pointer transition-colors duration-100 hover:bg-[var(--bg-hover)]" style={{ color: "var(--fill-secondary)" }}>
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

/* ━━━ Security Tab ━━━ */

type DangerousOpsPolicy = "deny" | "allow" | "confirm";

const POLICY_OPTIONS: { value: DangerousOpsPolicy; label: string; desc: string }[] = [
  { value: "deny", label: "拒绝", desc: "直接阻止所有危险操作" },
  { value: "confirm", label: "确认", desc: "暂停并弹窗询问用户是否继续（推荐）" },
  { value: "allow", label: "允许", desc: "不做任何检查，直接执行" },
];

function SecurityTab() {
  const [allowedHosts, setAllowedHosts] = useState<string[]>([]);
  const [newHost, setNewHost] = useState("");
  const [opsPolicy, setOpsPolicy] = useState<DangerousOpsPolicy>("confirm");
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [toast, setToast] = useState<{ msg: string; type: "ok" | "err" } | null>(null);

  const showToast = useCallback((msg: string, type: "ok" | "err") => {
    setToast({ msg, type });
    setTimeout(() => setToast(null), 2500);
  }, []);

  useEffect(() => {
    api.getConfig("security").then((data) => {
      const cfg = data as { key?: string; value?: Record<string, unknown> } | null;
      const val = (cfg?.value ?? cfg) as Record<string, unknown> | null;
      if (val?.ssrfAllowedHosts && Array.isArray(val.ssrfAllowedHosts)) {
        setAllowedHosts(val.ssrfAllowedHosts as string[]);
      }
      if (val?.dangerousOpsPolicy && typeof val.dangerousOpsPolicy === "string") {
        setOpsPolicy(val.dangerousOpsPolicy as DangerousOpsPolicy);
      }
    }).catch(() => {}).finally(() => setLoading(false));
  }, []);

  const persistSecurity = useCallback(async (patch: Record<string, unknown>) => {
    setSaving(true);
    try {
      await api.setConfig("security", patch);
      showToast("已保存，立即生效", "ok");
    } catch {
      showToast("保存失败", "err");
    } finally {
      setSaving(false);
    }
  }, [showToast]);

  const persistHosts = useCallback(async (hosts: string[]) => {
    setAllowedHosts(hosts);
    await persistSecurity({ ssrfAllowedHosts: hosts });
  }, [persistSecurity]);

  const handlePolicyChange = useCallback(async (policy: DangerousOpsPolicy) => {
    setOpsPolicy(policy);
    await persistSecurity({ dangerousOpsPolicy: policy });
  }, [persistSecurity]);

  const handleAdd = () => {
    const trimmed = newHost.trim();
    if (!trimmed || allowedHosts.includes(trimmed)) return;
    const updated = [...allowedHosts, trimmed];
    setNewHost("");
    persistHosts(updated);
  };

  const handleRemove = (host: string) => {
    persistHosts(allowedHosts.filter((h) => h !== host));
  };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Enter") {
      e.preventDefault();
      handleAdd();
    }
  };

  const inputCls = "w-full rounded-[6px] px-3 py-2 text-[13px] font-mono outline-none transition-all focus:ring-2 focus:ring-[var(--tint)]";
  const inputStyle: React.CSSProperties = { background: "var(--bg-base)", color: "var(--fill-primary)", border: "0.5px solid var(--separator-opaque)" };

  if (loading) {
    return (
      <div className="flex items-center justify-center py-12">
        <span className="text-[13px]" style={{ color: "var(--fill-tertiary)" }}>加载中...</span>
      </div>
    );
  }

  return (
    <div className="space-y-6">
      {toast && (
        <div
          className="flex items-center gap-2 rounded-[var(--radius-xs)] px-3 py-2 text-[12px] font-medium"
          style={{
            background: toast.type === "ok" ? "color-mix(in srgb, var(--green) 15%, transparent)" : "color-mix(in srgb, var(--red) 15%, transparent)",
            color: toast.type === "ok" ? "var(--green)" : "var(--red)",
            animation: "fade-in 0.15s ease-out",
          }}
        >
          {toast.type === "ok" ? <CheckCircle size={13} strokeWidth={1.5} /> : <XCircle size={13} strokeWidth={1.5} />}
          {toast.msg}
        </div>
      )}

      <div>
        <SectionTitle>危险操作保护</SectionTitle>
        <p className="mb-3 text-[11px]" style={{ color: "var(--fill-tertiary)" }}>
          控制 Shell 中执行 rm、rmdir、chmod 等危险命令时的行为策略。
        </p>
        <div className="overflow-hidden rounded-[var(--radius-sm)]" style={{ background: "var(--bg-elevated)", border: "0.5px solid var(--separator-opaque)" }}>
          {POLICY_OPTIONS.map((opt, idx) => (
            <button
              key={opt.value}
              onClick={() => handlePolicyChange(opt.value)}
              disabled={saving}
              className="flex w-full cursor-pointer items-center gap-3 px-4 py-3 text-left transition-colors duration-100 hover:bg-[var(--bg-hover)] disabled:cursor-not-allowed disabled:opacity-50"
              style={idx < POLICY_OPTIONS.length - 1 ? { borderBottom: "0.5px solid var(--separator)" } : undefined}
            >
              <span
                className="flex h-[18px] w-[18px] shrink-0 items-center justify-center rounded-full transition-all duration-150"
                style={{
                  border: opsPolicy === opt.value ? "none" : "1.5px solid var(--fill-quaternary)",
                  background: opsPolicy === opt.value ? "var(--tint)" : "transparent",
                }}
              >
                {opsPolicy === opt.value && (
                  <CheckCircle size={14} strokeWidth={2.5} style={{ color: "#fff" }} />
                )}
              </span>
              <div>
                <div className="text-[13px] font-medium" style={{ color: "var(--fill-primary)" }}>{opt.label}</div>
                <div className="text-[11px]" style={{ color: "var(--fill-tertiary)" }}>{opt.desc}</div>
              </div>
            </button>
          ))}
        </div>
      </div>

      <div>
        <SectionTitle>SSRF 白名单</SectionTitle>
        <p className="mb-3 text-[11px]" style={{ color: "var(--fill-tertiary)" }}>
          允许 http_fetch / web_fetch 访问的内网主机。默认情况下，解析到私有 IP (localhost, 10.x, 192.168.x) 的 URL 会被 SSRF 保护拦截。
          将主机名或 host:port 加入白名单后可绕过此限制，适用于本地 SearXNG、内部 API 等场景。
        </p>

        <div className="overflow-hidden rounded-[var(--radius-sm)]" style={{ background: "var(--bg-elevated)", border: "0.5px solid var(--separator-opaque)" }}>
          {allowedHosts.length === 0 ? (
            <div className="px-4 py-4 text-center text-[12px]" style={{ color: "var(--fill-tertiary)" }}>
              暂无白名单主机 — 所有指向私有 IP 的请求将被拦截
            </div>
          ) : (
            allowedHosts.map((host, idx) => (
              <div
                key={host}
                className="group flex items-center justify-between px-4 py-2.5 transition-colors duration-100 hover:bg-[var(--bg-hover)]"
                style={idx < allowedHosts.length - 1 ? { borderBottom: "0.5px solid var(--separator)" } : undefined}
              >
                <span className="text-[13px] font-mono" style={{ color: "var(--fill-primary)" }}>{host}</span>
                <button
                  onClick={() => handleRemove(host)}
                  disabled={saving}
                  className="flex h-6 w-6 shrink-0 cursor-pointer items-center justify-center rounded-full opacity-0 transition-all duration-100 hover:bg-[var(--bg-hover)] group-hover:opacity-100"
                  title="移除"
                >
                  <X size={12} strokeWidth={2} style={{ color: "var(--red)" }} />
                </button>
              </div>
            ))
          )}
        </div>

        <div className="mt-3 flex items-center gap-2">
          <input
            value={newHost}
            onChange={(e) => setNewHost(e.target.value)}
            onKeyDown={handleKeyDown}
            placeholder="例: localhost:8888 或 searxng.internal"
            className={inputCls}
            style={inputStyle}
            disabled={saving}
          />
          <button
            onClick={handleAdd}
            disabled={saving || !newHost.trim()}
            className="flex shrink-0 cursor-pointer items-center gap-1 rounded-[6px] px-3 py-2 text-[12px] font-medium text-white transition-colors hover:opacity-90 disabled:cursor-not-allowed disabled:opacity-50"
            style={{ background: "var(--tint)" }}
          >
            <Plus size={12} strokeWidth={2} />
            添加
          </button>
        </div>
      </div>

      <div>
        <p className="text-[11px]" style={{ color: "var(--fill-quaternary)" }}>
          配置保存到 ~/.fastclaw/config/default.json 的 security.ssrfAllowedHosts 字段，保存后立即生效。
        </p>
      </div>
    </div>
  );
}

/* ━━━ About Tab ━━━ */

function AboutTab() {
  const gwInfo = useGatewayStore((s) => s.info);
  return (
    <div className="space-y-6">
      <div className="flex flex-col items-center py-6">
        <div className="mb-4">
          <ClawIcon size={64} />
        </div>
        <h3 className="text-[16px] font-semibold" style={{ color: "var(--fill-primary)" }}>FastClaw</h3>
        <p className="mt-0.5 text-[12px]" style={{ color: "var(--fill-tertiary)" }}>
          版本 {gwInfo?.version ?? "0.1.0"}
        </p>
      </div>
      <div>
        <SectionTitle>信息</SectionTitle>
        {(() => {
          const rows = [
            { label: "框架", value: "Tauri 2.0 + React 19" },
            { label: "后端", value: "Rust (Tokio + Axum)" },
            { label: "协议", value: "fastclaw-ws/1 (WebSocket)" },
            { label: "许可证", value: "MIT" },
          ];
          return (
            <div className="overflow-hidden rounded-[var(--radius-sm)]" style={{ background: "var(--bg-elevated)", border: "0.5px solid var(--separator-opaque)" }}>
              {rows.map(({ label, value }, idx) => (
                <div key={label} className="flex items-center justify-between px-4 py-2.5" style={idx < rows.length - 1 ? { borderBottom: "0.5px solid var(--separator)" } : undefined}>
                  <span className="text-[13px]" style={{ color: "var(--fill-secondary)" }}>{label}</span>
                  <span className="text-[13px] font-medium" style={{ color: "var(--fill-primary)" }}>{value}</span>
                </div>
              ))}
            </div>
          );
        })()}
      </div>
    </div>
  );
}

/* ━━━ Main Settings Panel ━━━ */

const ICON_PROPS = { size: 16, strokeWidth: 1.5 } as const;
const TABS: { id: SettingsTab; label: string; icon: React.ReactNode }[] = [
  { id: "general", label: "通用", icon: <Settings2 {...ICON_PROPS} /> },
  { id: "models", label: "模型", icon: <Box {...ICON_PROPS} /> },
  { id: "web-search", label: "联网搜索", icon: <Search {...ICON_PROPS} /> },
  { id: "skills", label: "Skills", icon: <Wrench {...ICON_PROPS} /> },
  { id: "mcp", label: "MCP", icon: <Plug {...ICON_PROPS} /> },
  { id: "security", label: "安全", icon: <Shield {...ICON_PROPS} /> },
  { id: "gateway", label: "网关", icon: <Server {...ICON_PROPS} /> },
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
    <div className="fixed inset-0 z-50 flex items-center justify-center" style={{ animation: "fade-in 0.15s ease-out" }}>
      <div className="absolute inset-0" style={{ background: "rgba(0, 0, 0, 0.3)" }} onClick={onClose} />
      <div
        className="relative flex overflow-hidden rounded-[var(--radius-xl)]"
        style={{
          width: "min(720px, calc(100vw - 64px))",
          height: "min(520px, calc(100vh - 80px))",
          background: "var(--bg-elevated)",
          boxShadow: "var(--shadow-lg)",
          animation: "scale-in 0.2s ease-out",
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
            {tab === "general" && <GeneralTab />}
            {tab === "models" && <ModelsTab />}
            {tab === "web-search" && <WebSearchTab />}
            {tab === "skills" && <SkillsTab />}
            {tab === "mcp" && <McpTab />}
            {tab === "security" && <SecurityTab />}
            {tab === "gateway" && <GatewayTab />}
            {tab === "about" && <AboutTab />}
          </div>
        </div>
      </div>
    </div>
  );
}
