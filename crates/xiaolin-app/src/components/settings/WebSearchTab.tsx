import { useState, useEffect, useCallback } from "react";
import { Trans, useTranslation } from "react-i18next";
import { Eye, EyeSlash, Lightning, CheckCircle, XCircle, SpinnerGap, MagnifyingGlass } from "@phosphor-icons/react";
import * as api from "../../lib/api";
import { SectionTitle } from "./SettingsShared";
import { ICON_SIZE } from "../../lib/ui-tokens";
import { inputCls as sharedInputCls, inputStyle as sharedInputStyle, labelCls as sharedLabelCls, labelStyle as sharedLabelStyle } from "../common/FormElements";


type TestStatus = "idle" | "testing" | "success" | "error";


type WebSearchBackend = "tavily" | "searxng" | "builtin" | "";

const BUILTIN_ENGINE_IDS = ["google", "baidu", "bing", "sogou", "360"] as const;

export function WebSearchTab() {
  const { t } = useTranslation("settings");
  const builtinEngines = [
    { id: "google", label: "Google" },
    { id: "baidu", label: t("engine_baidu") },
    { id: "bing", label: "Bing" },
    { id: "sogou", label: t("engine_sogou") },
    { id: "360", label: t("engine_360") },
  ] as const;
  const [backend, setBackend] = useState<WebSearchBackend>("");
  const [tavilyKey, setTavilyKey] = useState("");
  const [searxngUrl, setSearxngUrl] = useState("");
  const [enabledEngines, setEnabledEngines] = useState<Set<string>>(new Set(BUILTIN_ENGINE_IDS));
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

      showToast(t("searchSavedRestart"), "ok");
    } catch {
      showToast(t("saveFailed"), "err");
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
          setTestMsg(t("testNeedApiKey"));
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
          setTestMsg(t("testTavilyOk"));
        } else {
          const body = await resp.text().catch(() => "");
          setTestStatus("error");
          setTestMsg(`HTTP ${resp.status}${body ? `: ${body.slice(0, 80)}` : ""}`);
        }
      } else if (backend === "searxng") {
        const base = searxngUrl.trim().replace(/\/+$/, "");
        if (!base) {
          setTestStatus("error");
          setTestMsg(t("testNeedSearxngUrl"));
          return;
        }
        const resp = await fetch(`${base}/search?q=test&format=json&categories=general`, {
          signal: AbortSignal.timeout(10000),
        });
        if (resp.ok) {
          setTestStatus("success");
          setTestMsg(t("testSearxngOk"));
        } else {
          setTestStatus("error");
          setTestMsg(`HTTP ${resp.status}`);
        }
      } else {
        setTestStatus("error");
        setTestMsg(t("testNeedEngine"));
      }
    } catch (err) {
      setTestStatus("error");
      const msg = typeof err === "string" ? err : err instanceof Error ? err.message : t("connectionFailed");
      setTestMsg(msg.length > 120 ? msg.slice(0, 120) + "…" : msg);
    }
  }, [backend, tavilyKey, searxngUrl]);

  if (loading) {
    return (
      <div className="flex items-center justify-center py-12">
        <span className="text-[13px]" style={{ color: "var(--fill-tertiary)" }}>{t("loading")}</span>
      </div>
    );
  }

  const inputCls = sharedInputCls;
  const inputStyle = sharedInputStyle;
  const labelCls = sharedLabelCls;
  const labelStyle = sharedLabelStyle;

  return (
    <div className="space-y-6">
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

      <div>
        <SectionTitle>{t("searchEngine")}</SectionTitle>
        <p className="mb-3 text-[12px]" style={{ color: "var(--fill-tertiary)" }}>
          {t("searchEngineDesc")}
        </p>
        <div className="overflow-hidden rounded-[var(--radius-sm)]" style={{ background: "var(--bg-elevated)", border: "0.5px solid var(--separator-opaque)" }}>
          {([
            { value: "builtin" as const, label: t("searchBuiltin"), desc: t("searchBuiltinDesc") },
            { value: "tavily" as const, label: "Tavily", desc: t("searchTavilyDesc") },
            { value: "searxng" as const, label: "SearXNG", desc: t("searchSearxngDesc") },
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
                  transition: "all var(--duration-fast)",
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
          <SectionTitle>{t("searchEngineSelect")}</SectionTitle>
          <p className="mb-3 text-[11px]" style={{ color: "var(--fill-tertiary)" }}>
            {t("searchEngineSelectDesc")}
          </p>
          <div className="overflow-hidden rounded-[var(--radius-sm)]" style={{ background: "var(--bg-elevated)", border: "0.5px solid var(--separator-opaque)" }}>
            {builtinEngines.map((eng, idx) => {
              const checked = enabledEngines.has(eng.id);
              return (
                <div
                  key={eng.id}
                  className="flex cursor-pointer items-center gap-3 px-4 py-2.5 transition-colors duration-100 hover:bg-[var(--bg-hover)]"
                  style={idx < builtinEngines.length - 1 ? { borderBottom: "0.5px solid var(--separator)" } : undefined}
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
                    className="flex h-[18px] w-[18px] shrink-0 items-center justify-center rounded-[var(--radius-xs)]"
                    style={{
                      border: `2px solid ${checked ? "var(--tint)" : "var(--fill-quaternary)"}`,
                      background: checked ? "var(--tint)" : "transparent",
                      transition: "all var(--duration-fast)",
                    }}
                  >
                    {checked && (
                      <svg width="12" height="12" viewBox="0 0 10 10" fill="none" xmlns="http://www.w3.org/2000/svg">
                        <path d="M2 5L4.2 7.5L8 2.5" stroke="white" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" />
                      </svg>
                    )}
                  </div>
                  <span className="text-[13px]" style={{ color: "var(--fill-primary)" }}>{eng.label}</span>
                </div>
              );
            })}
          </div>
          <p className="mt-2 text-[11px]" style={{ color: "var(--fill-quaternary)" }}>
            {t("searchMinOne", { selected: enabledEngines.size, total: builtinEngines.length })}
          </p>
        </div>
      )}

      {backend === "tavily" && (
        <div>
          <SectionTitle>{t("tavilyConfig")}</SectionTitle>
          <div className="space-y-3">
            <div>
              <label className={labelCls} style={labelStyle}>{t("apiKey")}</label>
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
                    className="flex h-7 w-7 cursor-pointer items-center justify-center rounded-[var(--radius-xs)] transition-colors hover:bg-[var(--bg-hover)]"
                  >
                    {showKey
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
            <p className="text-[11px]" style={{ color: "var(--fill-quaternary)" }}>
              <Trans i18nKey="tavilySignup" ns="settings" components={{ 1: <a href="https://tavily.com" target="_blank" rel="noreferrer" className="underline" style={{ color: "var(--tint)" }} /> }} />
            </p>
          </div>
        </div>
      )}

      {backend === "searxng" && (
        <div>
          <SectionTitle>{t("searxngConfig")}</SectionTitle>
          <div className="space-y-3">
            <div>
              <label className={labelCls} style={labelStyle}>{t("searxngInstanceUrl")}</label>
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
                  className="absolute top-1/2 right-2 flex h-7 -translate-y-1/2 cursor-pointer items-center gap-1 rounded-[var(--radius-xs)] px-1.5 text-[11px] font-medium transition-colors hover:bg-[var(--bg-hover)] disabled:opacity-50"
                  style={{ color: testStatus === "success" ? "var(--green)" : testStatus === "error" ? "var(--red)" : "var(--tint)" }}
                >
                  {testStatus === "testing" ? <SpinnerGap size={ICON_SIZE.md} className="animate-spin" />
                    : testStatus === "success" ? <CheckCircle size={ICON_SIZE.md} />
                    : testStatus === "error" ? <XCircle size={ICON_SIZE.md} />
                    : <Lightning size={ICON_SIZE.md} />
                  }
                  {testStatus === "idle" && t("test")}
                </button>
              </div>
              {testMsg && (
                <p className="mt-1.5 text-[11px]" style={{ color: testStatus === "success" ? "var(--green)" : "var(--red)" }}>
                  {testMsg}
                </p>
              )}
            </div>
            <p className="text-[11px]" style={{ color: "var(--fill-quaternary)" }}>
              {t("searxngJsonHint")}
            </p>
          </div>
        </div>
      )}

      {!backend && (
        <div className="rounded-[var(--radius-sm)] px-4 py-6 text-center" style={{ background: "var(--bg-elevated)", border: "0.5px solid var(--separator-opaque)" }}>
          <MagnifyingGlass size={24} className="mx-auto mb-2" style={{ color: "var(--fill-quaternary)" }} />
          <p className="text-[13px]" style={{ color: "var(--fill-tertiary)" }}>
            {t("searchSelectBackend")}
          </p>
        </div>
      )}

      <div className="flex items-center justify-end gap-2">
        <button
          onClick={handleSave}
          disabled={saving}
          className="rounded-[var(--radius-xs)] px-4 py-2 text-[13px] font-medium text-white transition-colors hover:opacity-90 disabled:opacity-50"
          style={{ background: "var(--tint)", cursor: saving ? "not-allowed" : "pointer" }}
        >
          {saving ? t("saving") : t("save")}
        </button>
      </div>

      <p className="text-[11px]" style={{ color: "var(--fill-quaternary)" }}>
        {t("searchConfigHint")}
      </p>
    </div>
  );
}