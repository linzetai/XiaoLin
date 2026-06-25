import { useState, useCallback, useEffect } from "react";
import { useTranslation } from "react-i18next";
import { Plus, Trash, FloppyDisk, X } from "@phosphor-icons/react";
import { isTauri } from "../../lib/transport";
import { invoke } from "@tauri-apps/api/core";

export type BrowserProxyMode = "none" | "system" | "custom" | "xiaolin_proxy";

export interface HostMappingEntry {
  pattern: string;
  targetIp: string;
}

export interface BrowserNetworkConfig {
  proxyMode: BrowserProxyMode;
  customProxyUrl?: string | null;
  upstreamProxyUrl?: string | null;
  hostMappings: HostMappingEntry[];
  sessionHostMappings: HostMappingEntry[];
}

interface BrowserNetworkSettingsProps {
  open: boolean;
  onClose: () => void;
}

function emptyMapping(): HostMappingEntry {
  return { pattern: "", targetIp: "" };
}

function fromBackend(raw: Record<string, unknown>): BrowserNetworkConfig {
  const mapEntry = (m: Record<string, unknown>): HostMappingEntry => ({
    pattern: String(m.pattern ?? ""),
    targetIp: String(m.target_ip ?? m.targetIp ?? ""),
  });
  const hostMappings = Array.isArray(raw.host_mappings)
    ? (raw.host_mappings as Record<string, unknown>[]).map(mapEntry)
    : [];
  const sessionHostMappings = Array.isArray(raw.session_host_mappings)
    ? (raw.session_host_mappings as Record<string, unknown>[]).map(mapEntry)
    : [];
  return {
    proxyMode: (raw.proxy_mode as BrowserProxyMode) ?? "xiaolin_proxy",
    customProxyUrl: (raw.custom_proxy_url as string | null) ?? null,
    upstreamProxyUrl: (raw.upstream_proxy_url as string | null) ?? null,
    hostMappings,
    sessionHostMappings,
  };
}

function toBackend(cfg: BrowserNetworkConfig): Record<string, unknown> {
  return {
    proxy_mode: cfg.proxyMode,
    custom_proxy_url: cfg.customProxyUrl || null,
    upstream_proxy_url: cfg.upstreamProxyUrl || null,
    host_mappings: cfg.hostMappings
      .filter((m) => m.pattern.trim() && m.targetIp.trim())
      .map((m) => ({ pattern: m.pattern.trim(), target_ip: m.targetIp.trim() })),
    session_host_mappings: cfg.sessionHostMappings.map((m) => ({
      pattern: m.pattern,
      target_ip: m.targetIp,
    })),
  };
}

async function loadConfig(): Promise<BrowserNetworkConfig> {
  if (!isTauri) {
    return {
      proxyMode: "xiaolin_proxy",
      hostMappings: [],
      sessionHostMappings: [],
    };
  }
  const raw = await invoke<string>("browser_get_network_config");
  return fromBackend(JSON.parse(raw) as Record<string, unknown>);
}

async function saveConfig(cfg: BrowserNetworkConfig): Promise<void> {
  if (!isTauri) return;
  await invoke("browser_save_network_config", { config: toBackend(cfg) });
}

const PROXY_MODE_IDS: BrowserProxyMode[] = ["xiaolin_proxy", "none", "system", "custom"];

const proxyModeKeys: Record<BrowserProxyMode, { label: string; hint: string }> = {
  xiaolin_proxy: { label: "proxyXiaolinLabel", hint: "proxyXiaolinHint" },
  none: { label: "proxyNoneLabel", hint: "proxyNoneHint" },
  system: { label: "proxySystemLabel", hint: "proxySystemHint" },
  custom: { label: "proxyCustomLabel", hint: "proxyCustomHint" },
};

export function BrowserNetworkSettings({ open, onClose }: BrowserNetworkSettingsProps) {
  const { t } = useTranslation("browser");
  const [config, setConfig] = useState<BrowserNetworkConfig | null>(null);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [loaded, setLoaded] = useState(false);

  useEffect(() => {
    if (!open) return;
    setLoaded(false);
    setError(null);
    loadConfig()
      .then((cfg) => {
        setConfig(cfg);
        setLoaded(true);
      })
      .catch((e) => {
        console.warn("[browser-network] load config failed:", e);
        setError("loadConfigFailed");
        setLoaded(true);
      });
  }, [open]);

  const updateMapping = useCallback((index: number, patch: Partial<HostMappingEntry>) => {
    setConfig((prev) => {
      if (!prev) return prev;
      const hostMappings = [...prev.hostMappings];
      hostMappings[index] = { ...hostMappings[index], ...patch };
      return { ...prev, hostMappings };
    });
  }, []);

  const addMapping = useCallback(() => {
    setConfig((prev) => {
      if (!prev) return prev;
      return { ...prev, hostMappings: [...prev.hostMappings, emptyMapping()] };
    });
  }, []);

  const removeMapping = useCallback((index: number) => {
    setConfig((prev) => {
      if (!prev) return prev;
      return {
        ...prev,
        hostMappings: prev.hostMappings.filter((_, i) => i !== index),
      };
    });
  }, []);

  const handleSave = useCallback(async () => {
    if (!config) return;
    setSaving(true);
    setError(null);
    try {
      await saveConfig(config);
      onClose();
    } catch (e) {
      console.warn("[browser-network] save config failed:", e);
      setError("saveConfigFailed");
    } finally {
      setSaving(false);
    }
  }, [config, onClose]);

  if (!open) return null;

  return (
    <div
      className="fixed inset-0 z-[60] flex items-center justify-center"
      style={{ background: "rgba(0,0,0,0.35)" }}
      onClick={onClose}
    >
      <div
        className="relative flex max-h-[85vh] w-[min(520px,calc(100vw-32px))] flex-col overflow-hidden rounded-xl"
        style={{
          background: "var(--bg-elevated)",
          border: "0.5px solid var(--separator)",
          boxShadow: "var(--shadow-lg)",
        }}
        onClick={(e) => e.stopPropagation()}
      >
        <div
          className="flex shrink-0 items-center justify-between px-5 py-4"
          style={{ borderBottom: "0.5px solid var(--separator)" }}
        >
          <h3 className="text-[15px] font-semibold" style={{ color: "var(--fill-primary)" }}>
            {t("networkConfig")}
          </h3>
          <button
            type="button"
            onClick={onClose}
            className="cursor-pointer rounded-md p-1"
            style={{ color: "var(--fill-tertiary)" }}
            title={t("closeDialog")}
            aria-label={t("closeDialog")}
          >
            <X size={18} />
          </button>
        </div>

        <div className="min-h-0 flex-1 overflow-y-auto px-5 py-4">
          {!loaded && (
            <div className="text-[13px]" style={{ color: "var(--fill-secondary)" }}>
              {t("loadingConfig")}
            </div>
          )}
          {error && (
            <div
              className="mb-3 rounded-md px-3 py-2 text-[12px]"
              style={{ background: "var(--color-red-900, rgba(127,29,29,0.2))", color: "var(--color-red-300, #fca5a5)" }}
            >
              {t(error)}
            </div>
          )}
          {loaded && config && (
            <>
              <div className="mb-4">
                <div className="mb-2 text-[12px] font-medium" style={{ color: "var(--fill-secondary)" }}>
                  {t("proxyMode")}
                </div>
                <div className="flex flex-col gap-2">
                  {PROXY_MODE_IDS.map((modeId) => (
                    <label
                      key={modeId}
                      className="flex cursor-pointer items-start gap-2 rounded-lg px-3 py-2"
                      style={{
                        background:
                          config.proxyMode === modeId ? "var(--bg-active)" : "var(--bg-secondary)",
                        border: "0.5px solid var(--separator)",
                      }}
                    >
                      <input
                        type="radio"
                        name="proxyMode"
                        checked={config.proxyMode === modeId}
                        onChange={() => setConfig({ ...config, proxyMode: modeId })}
                        className="mt-0.5"
                      />
                      <span>
                        <span className="block text-[13px] font-medium" style={{ color: "var(--fill-primary)" }}>
                          {t(proxyModeKeys[modeId].label)}
                        </span>
                        <span className="text-[11px]" style={{ color: "var(--fill-tertiary)" }}>
                          {t(proxyModeKeys[modeId].hint)}
                        </span>
                      </span>
                    </label>
                  ))}
                </div>
              </div>

              {config.proxyMode === "custom" && (
                <div className="mb-4">
                  <label className="mb-1 block text-[12px]" style={{ color: "var(--fill-secondary)" }}>
                    {t("customProxyUrl")}
                  </label>
                  <input
                    type="text"
                    value={config.customProxyUrl ?? ""}
                    onChange={(e) => setConfig({ ...config, customProxyUrl: e.target.value })}
                    placeholder={t("customProxyPlaceholder")}
                    className="w-full rounded-md px-3 py-2 text-[13px]"
                    style={{
                      background: "var(--bg-secondary)",
                      border: "0.5px solid var(--separator)",
                      color: "var(--fill-primary)",
                    }}
                  />
                </div>
              )}

              {config.proxyMode === "xiaolin_proxy" && (
                <div className="mb-4">
                  <label className="mb-1 block text-[12px]" style={{ color: "var(--fill-secondary)" }}>
                    {t("upstreamProxy")}
                  </label>
                  <input
                    type="text"
                    value={config.upstreamProxyUrl ?? ""}
                    onChange={(e) => setConfig({ ...config, upstreamProxyUrl: e.target.value })}
                    placeholder={t("upstreamProxyPlaceholder")}
                    className="w-full rounded-md px-3 py-2 text-[13px]"
                    style={{
                      background: "var(--bg-secondary)",
                      border: "0.5px solid var(--separator)",
                      color: "var(--fill-primary)",
                    }}
                  />
                </div>
              )}

              <div>
                <div className="mb-2 flex items-center justify-between">
                  <span className="text-[12px] font-medium" style={{ color: "var(--fill-secondary)" }}>
                    {t("hostMapping")}
                  </span>
                  <button
                    type="button"
                    onClick={addMapping}
                    className="flex cursor-pointer items-center gap-1 text-[12px]"
                    style={{ color: "var(--accent)" }}
                  >
                    <Plus size={14} />
                    {t("addMapping")}
                  </button>
                </div>
                <p className="mb-2 text-[11px]" style={{ color: "var(--fill-tertiary)" }}>
                  {t("hostMappingHint")}
                </p>
                {config.hostMappings.length === 0 && (
                  <div className="rounded-md px-3 py-4 text-center text-[12px]" style={{ color: "var(--fill-tertiary)", background: "var(--bg-secondary)" }}>
                    {t("noMapping")}
                  </div>
                )}
                <div className="flex flex-col gap-2">
                  {config.hostMappings.map((m, i) => (
                    <div key={i} className="flex gap-2">
                      <input
                        type="text"
                        value={m.pattern}
                        onChange={(e) => updateMapping(i, { pattern: e.target.value })}
                        placeholder={t("hostPatternPlaceholder")}
                        className="min-w-0 flex-1 rounded-md px-2 py-1.5 text-[12px]"
                        style={{
                          background: "var(--bg-secondary)",
                          border: "0.5px solid var(--separator)",
                          color: "var(--fill-primary)",
                        }}
                      />
                      <span className="self-center text-[12px]" style={{ color: "var(--fill-tertiary)" }}>
                        →
                      </span>
                      <input
                        type="text"
                        value={m.targetIp}
                        onChange={(e) => updateMapping(i, { targetIp: e.target.value })}
                        placeholder={t("targetIpPlaceholder")}
                        className="min-w-0 flex-1 rounded-md px-2 py-1.5 text-[12px]"
                        style={{
                          background: "var(--bg-secondary)",
                          border: "0.5px solid var(--separator)",
                          color: "var(--fill-primary)",
                        }}
                      />
                      <button
                        type="button"
                        onClick={() => removeMapping(i)}
                        className="cursor-pointer rounded-md p-1.5"
                        style={{ color: "var(--fill-tertiary)" }}
                        aria-label={t("removeMapping")}
                      >
                        <Trash size={14} />
                      </button>
                    </div>
                  ))}
                </div>
              </div>

              {config.sessionHostMappings.length > 0 && (
                <div className="mt-4 rounded-md px-3 py-2 text-[11px]" style={{ background: "var(--bg-secondary)", color: "var(--fill-tertiary)" }}>
                  {t("sessionMappings", { count: config.sessionHostMappings.length })}
                </div>
              )}
            </>
          )}
        </div>

        <div
          className="flex shrink-0 justify-end gap-2 px-5 py-3"
          style={{ borderTop: "0.5px solid var(--separator)" }}
        >
          <button
            type="button"
            onClick={onClose}
            className="cursor-pointer rounded-md px-4 py-2 text-[13px]"
            style={{ color: "var(--fill-secondary)" }}
          >
            {t("common:cancel")}
          </button>
          <button
            type="button"
            onClick={() => void handleSave()}
            disabled={saving || !config}
            className="flex cursor-pointer items-center gap-1.5 rounded-md px-4 py-2 text-[13px] font-medium disabled:opacity-50"
            style={{ background: "var(--accent)", color: "var(--accent-fg, #fff)" }}
          >
            <FloppyDisk size={16} />
            {saving ? t("saving") : t("common:save")}
          </button>
        </div>
      </div>
    </div>
  );
}

export { loadConfig as loadBrowserNetworkConfig, saveConfig as saveBrowserNetworkConfig };
