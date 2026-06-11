import { useState, useEffect, useCallback } from "react";
import { useTranslation } from "react-i18next";
import { CaretDown, Plus, PencilSimple, X, Eye, EyeSlash, Lightning, CheckCircle, XCircle, SpinnerGap, Trash } from "@phosphor-icons/react";
import * as api from "../../lib/api";
import { SectionTitle } from "./SettingsShared";
import { inferContextWindow, takeModelSnapshot, popModelSnapshot, hasModelSnapshots } from "../../lib/model-registry";
import { useModelTest } from "../../lib/model-utils";
import { ICON_SIZE } from "../../lib/ui-tokens";
import { inputCls as sharedInputCls, inputStyle as sharedInputStyle, labelCls as sharedLabelCls, labelStyle as sharedLabelStyle, FormButton } from "../common/FormElements";


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

function ModelFormModal({
  t,
  entry,
  credential,
  isNew,
  onSave,
  onCancel,
  onDelete,
  saving,
}: {
  t: (key: string, opts?: Record<string, unknown>) => string;
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
  const [showAdvanced, setShowAdvanced] = useState(false);
  const { testStatus, testMsg, runTest, resetTest } = useModelTest();
  const patch = (k: string, v: string | number) => setForm((f) => ({ ...f, [k]: v }));

  useEffect(() => {
    const handleKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") { e.stopPropagation(); onCancel(); }
    };
    window.addEventListener("keydown", handleKey);
    return () => window.removeEventListener("keydown", handleKey);
  }, [onCancel]);

  const handleTest = () => {
    const baseUrl = (form.baseUrl || cred.baseUrl || "").replace(/\/+$/, "");
    runTest(baseUrl, cred.apiKey, form.model || undefined);
  };

  const inputCls = sharedInputCls;
  const inputStyle = sharedInputStyle;
  const labelCls = sharedLabelCls;
  const labelStyle = sharedLabelStyle;

  return (
    <div className="fixed inset-0 z-[60] flex items-center justify-center" onClick={onCancel}>
      <div className="absolute inset-0" style={{ background: "rgba(0,0,0,0.25)" }} />
      <div
        className="relative w-full max-w-[480px] overflow-hidden rounded-[var(--radius-lg)]"
        style={{ background: "var(--bg-elevated)", boxShadow: "var(--shadow-lg)", border: "0.5px solid var(--separator-opaque)", animation: "scale-in var(--duration-fast) var(--ease-out)" }}
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center justify-between px-5 py-4" style={{ borderBottom: "0.5px solid var(--separator)" }}>
          <h3 className="text-[14px] font-semibold" style={{ color: "var(--fill-primary)" }}>
            {isNew ? t("modelAdd") : t("modelEdit", { key: entry.key })}
          </h3>
          <button onClick={onCancel} className="flex h-7 w-7 cursor-pointer items-center justify-center rounded-full transition-colors hover:bg-[var(--bg-hover)]" style={{ color: "var(--fill-tertiary)" }}>
            <X size={ICON_SIZE.md} />
          </button>
        </div>
        <div className="max-h-[60vh] space-y-4 overflow-y-auto px-5 py-4">
          <div className="grid grid-cols-2 gap-3">
            <div>
              <label className={labelCls} style={labelStyle}>{t("modelNameKey")}</label>
              <input value={form.key} onChange={(e) => patch("key", e.target.value)} disabled={!isNew} placeholder={t("placeholderKey")} className={inputCls} style={{ ...inputStyle, opacity: isNew ? 1 : 0.6 }} />
            </div>
            <div>
              <label className={labelCls} style={labelStyle}>Provider</label>
              <div className="relative">
                <select value={form.provider} onChange={(e) => patch("provider", e.target.value)} className="select-premium select-mono">
                  <option value="openai_compatible">OpenAI Compatible</option>
                  <option value="openai">OpenAI</option>
                  <option value="anthropic">Anthropic</option>
                </select>
                <CaretDown  className="pointer-events-none absolute top-1/2 right-3 -translate-y-1/2" style={{ color: "var(--fill-tertiary)" }} />
              </div>
            </div>
          </div>
          <div className="grid grid-cols-2 gap-3">
            <div>
              <label className={labelCls} style={labelStyle}>{t("modelName")}</label>
              <input value={form.model} onChange={(e) => {
                const modelId = e.target.value;
                patch("model", modelId);
                if (form.contextWindow === 0 && modelId) {
                  const inferred = inferContextWindow(modelId);
                  if (inferred !== 8192) patch("contextWindow", inferred);
                }
              }} placeholder={t("placeholderModel")} className={inputCls} style={inputStyle} />
            </div>
            <div>
              <label className={labelCls} style={labelStyle}>Base URL</label>
              <input value={form.baseUrl} onChange={(e) => patch("baseUrl", e.target.value)} placeholder="https://api.openai.com/v1" className={inputCls} style={inputStyle} />
            </div>
          </div>
          <div>
            <label className={labelCls} style={labelStyle}>
              {t("contextWindow")} <span style={{ color: "var(--red, #FC8181)" }}>*</span>
            </label>
            <input
              type="number"
              min="1024"
              step="1024"
              value={form.contextWindow || ""}
              onChange={(e) => patch("contextWindow", parseInt(e.target.value) || 0)}
              placeholder={t("placeholderContext")}
              required
              className={inputCls}
              style={{
                ...inputStyle,
                borderColor: form.contextWindow <= 0 ? "var(--red, #FC8181)" : undefined,
              }}
            />
            <p className="mt-1 text-[10px]" style={{ color: form.contextWindow <= 0 ? "var(--red, #FC8181)" : "var(--fill-quaternary)" }}>
              {form.contextWindow <= 0 ? t("contextWindowRequired") : t("contextWindowHint")}
            </p>
          </div>
          <div>
            <label className={labelCls} style={labelStyle}>API Key</label>
            <div className="relative">
              <input
                type={showApiKey ? "text" : "password"}
                value={cred.apiKey}
                onChange={(e) => { setCred((c) => ({ ...c, apiKey: e.target.value })); if (testStatus !== "idle") resetTest(); }}
                placeholder="sk-..."
                className={`${inputCls} pr-20 font-mono`}
                style={inputStyle}
              />
              <div className="absolute top-1/2 right-2 flex -translate-y-1/2 items-center gap-1">
                <button
                  type="button"
                  onClick={() => setShowApiKey((v) => !v)}
                  className="flex h-7 w-7 cursor-pointer items-center justify-center rounded-[var(--radius-xs)] transition-colors hover:bg-[var(--bg-hover)]"
                  title={showApiKey ? t("hideApiKey") : t("showApiKey")}
                >
                  {showApiKey
                    ? <EyeSlash size={ICON_SIZE.md} style={{ color: "var(--fill-tertiary)" }} />
                    : <Eye size={ICON_SIZE.md} style={{ color: "var(--fill-tertiary)" }} />
                  }
                </button>
                <button
                  type="button"
                  onClick={handleTest}
                  disabled={testStatus === "testing"}
                  className="flex h-7 cursor-pointer items-center gap-1 rounded-[var(--radius-xs)] px-1.5 text-[11px] font-medium transition-colors hover:bg-[var(--bg-hover)] disabled:opacity-50"
                  style={{ color: testStatus === "success" ? "var(--green)" : testStatus === "error" ? "var(--red)" : "var(--tint)" }}
                  title={t("testConnection")}
                >
                  {testStatus === "testing" ? <SpinnerGap size={ICON_SIZE.md} className="animate-spin" />
                    : testStatus === "success" ? <CheckCircle size={ICON_SIZE.md} />
                    : testStatus === "error" ? <XCircle size={ICON_SIZE.md} />
                    : <Lightning size={ICON_SIZE.md} />
                  }
                  {testStatus === "idle" && t("test")}
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
              <CaretDown  style={{ transform: showAdvanced ? "rotate(180deg)" : "rotate(0)", transition: "transform var(--duration-fast)" }} />
              {t("advancedSettings")}
            </button>
            {showAdvanced && (
              <div className="mt-3 space-y-3">
                {/* Temperature preset selector */}
                <div>
                  <label className={labelCls} style={{ ...labelStyle, display: "flex", alignItems: "center", gap: 4 }}>
                    {t("temperature")}
                    <span style={{ color: "var(--fill-quaternary)", fontWeight: 400 }}>
                      ({form.temperature})
                    </span>
                  </label>
                  {(() => {
                    const TIERS: Array<{ label: string; value: number; desc: string }> = [
                      { label: t("temp_precise"), value: 0, desc: t("temp_preciseDesc") },
                      { label: t("temp_balanced"), value: 0.7, desc: t("temp_balancedDesc") },
                      { label: t("temp_creative"), value: 1.0, desc: t("temp_creativeDesc") },
                      { label: t("temp_free"), value: 1.5, desc: t("temp_freeDesc") },
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
                                transition: "all var(--duration-fast)",
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
                    <span style={{ fontSize: 10, color: "var(--fill-quaternary)" }}>{t("tempCustom")}</span>
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
                    <label className={labelCls} style={labelStyle}>{t("concurrency")}</label>
                    <input type="number" min="1" value={form.maxConcurrent} onChange={(e) => patch("maxConcurrent", parseInt(e.target.value) || 1)} className={inputCls} style={inputStyle} />
                  </div>
                  <div>
                    <label className={labelCls} style={labelStyle}>{t("timeoutSecs")}</label>
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
                className="flex cursor-pointer items-center gap-1 rounded-[var(--radius-xs)] px-3 py-1.5 text-[12px] font-medium transition-colors hover:opacity-80"
                style={{ color: "var(--red)" }}
              >
                <Trash size={ICON_SIZE.md} />
                {t("delete")}
              </button>
            )}
          </div>
          <div className="flex items-center gap-2">
            <FormButton variant="ghost" onClick={onCancel} disabled={saving}>
              {t("cancel")}
            </FormButton>
            <FormButton
              variant="primary"
              onClick={() => onSave(form, cred)}
              disabled={saving || !form.key || !form.model}
            >
              {saving ? t("saving") : t("save")}
            </FormButton>
          </div>
        </div>
      </div>
    </div>
  );
}

export function ModelTab() {
  const { t } = useTranslation("settings");
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
      alert(t("contextWindowAlert"));
      return;
    }
    setSaving(true);
    takeModelSnapshot(modelsConfig, credentials as unknown as Record<string, Record<string, unknown>>);
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
      window.dispatchEvent(new CustomEvent("xiaolin:models-updated"));
      showToast(t("modelSaved"), "ok");
    } catch {
      showToast(t("saveFailed"), "err");
    } finally {
      setSaving(false);
    }
  };

  const handleDelete = async (key: string) => {
    setSaving(true);
    takeModelSnapshot(modelsConfig, credentials as unknown as Record<string, Record<string, unknown>>);
    try {
      const newModels = { ...modelsConfig };
      delete newModels[key];
      await api.setConfig("models", newModels);

      const defaultModelCfg = await api.getConfig("agents") as { value?: { defaults?: { model?: string } } } | null;
      const currentDefault = defaultModelCfg?.value?.defaults?.model ?? "";
      if (currentDefault && currentDefault.startsWith(`${key}/`)) {
        await api.setConfig("agents.defaults.model", "");
      }

      const remainingProviders = new Set(
        Object.values(newModels).map((v) => (v as Record<string, unknown>).provider ?? (v as Record<string, unknown>).key).filter(Boolean)
      );
      if (!remainingProviders.has(key) && credentials[key]) {
        const newCreds = { ...credentials } as Record<string, unknown>;
        delete newCreds[key];
        await api.setConfig("credentials", newCreds);
      }

      setEditing(null);
      loadData();
      window.dispatchEvent(new CustomEvent("xiaolin:models-updated"));
      showToast(t("modelDeleted", { key }), "ok");
    } catch {
      showToast(t("deleteFailed"), "err");
    } finally {
      setSaving(false);
    }
  };

  const handleRollback = async () => {
    const snapshot = popModelSnapshot();
    if (!snapshot) return;
    setSaving(true);
    try {
      await api.setConfig("models", snapshot.models);
      await api.setConfig("credentials", snapshot.credentials);
      loadData();
      window.dispatchEvent(new CustomEvent("xiaolin:models-updated"));
      showToast(t("rolledBack"), "ok");
    } catch {
      showToast(t("rollbackFailed"), "err");
    } finally {
      setSaving(false);
    }
  };

  if (loading) {
    return (
      <div className="flex items-center justify-center py-12">
        <span className="text-[13px]" style={{ color: "var(--fill-tertiary)" }}>{t("loading")}</span>
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
          }}
        >
          {toast.type === "ok" ? <CheckCircle size={ICON_SIZE.md} /> : <XCircle size={ICON_SIZE.md} />}
          {toast.msg}
        </div>
      )}
      <div className="flex items-center justify-between">
        <SectionTitle>{t("configuredModels", { count: entries.length })}</SectionTitle>
        <div className="flex items-center gap-2">
          {hasModelSnapshots() && (
            <button
              onClick={handleRollback}
              disabled={saving}
              className="flex cursor-pointer items-center gap-1 rounded-[var(--radius-xs)] px-2.5 py-1 text-[12px] font-medium transition-colors hover:opacity-80 disabled:opacity-40"
              style={{ color: "var(--fill-tertiary)" }}
            >
              {t("undo")}
            </button>
          )}
          <button
            onClick={() => { setAdding(true); setEditing(null); }}
            className="flex cursor-pointer items-center gap-1 rounded-[var(--radius-xs)] px-2.5 py-1 text-[12px] font-medium transition-colors hover:opacity-80"
            style={{ color: "var(--tint)" }}
          >
            <Plus size={ICON_SIZE.md} />
            {t("modelAdd")}
          </button>
        </div>
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
                    <span style={{ color: "var(--fill-tertiary)" }}>{t("apiKeyConfigured")}</span>
                  </div>
                )}
              </div>
              <PencilSimple size={ICON_SIZE.md} className="shrink-0 opacity-0 transition-opacity group-hover:opacity-100" style={{ color: "var(--fill-quaternary)" }} />
            </div>
          </div>
        ))}
      </div>

      {entries.length === 0 && (
        <div className="py-8 text-center">
          <p className="text-[13px]" style={{ color: "var(--fill-tertiary)" }}>
            {t("noModels")}
          </p>
        </div>
      )}

      <p className="text-[11px]" style={{ color: "var(--fill-quaternary)" }}>
        {t("modelEditHint")}
      </p>

      {(editing || adding) && (
        <ModelFormModal
          t={t}
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