import { useState, useEffect, useCallback } from "react";
import { ChevronDown, Plus, Pencil, X, Eye, EyeOff, Zap, CheckCircle, XCircle, Loader2, Trash2 } from "lucide-react";
import * as api from "../../lib/api";
import * as transport from "../../lib/transport";
import { SectionTitle } from "./SettingsShared";


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

export function ModelTab() {
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