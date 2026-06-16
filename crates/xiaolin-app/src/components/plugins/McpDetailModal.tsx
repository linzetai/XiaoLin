import { useState, useEffect, useCallback, useMemo } from "react";
import type { Icon } from "@phosphor-icons/react";
import {
  X, ArrowsClockwise, Trash, Wrench, SpinnerGap,
  WarningCircle, CheckCircle, WifiSlash, Clock,
  Terminal, Globe, Lightning, CaretDown, CaretRight,
  MagnifyingGlass, PencilSimple, PuzzlePiece,
  FolderOpen, GithubLogo, Database, Browser, ChatCircle,
  Brain, TreeStructure, Cube, MapPin,
  Package, GitBranch, ChatText, Files,
} from "@phosphor-icons/react";
import { useTranslation } from "react-i18next";
import { usePluginStore } from "../../lib/stores/plugin-store";
import * as transport from "../../lib/transport";
import type { McpDetailResult, McpPromptInfo, McpResourceInfo } from "../../lib/transport";
import { ICON_SIZE, BTN_PRIMARY_SM } from "../../lib/ui-tokens";
import { registry } from "./McpExplorePanel";
import type { McpRegistryEntry } from "./McpExplorePanel";

interface McpDetailModalProps {
  open: boolean;
  pluginId: string | null;
  onClose: () => void;
  onEditConfig?: (pluginId: string) => void;
}

const ICON_MAP: Record<string, Icon> = {
  FolderOpen, GithubLogo, Database, Browser, ChatCircle,
  Brain, TreeStructure, Cube, Globe, MapPin, Clock,
  Package, GitBranch, MagnifyingGlass, PuzzlePiece,
};

const STATUS_CONFIG: Record<string, { icon: typeof CheckCircle; color: string; labelKey: string }> = {
  connected: { icon: CheckCircle, color: "var(--green)", labelKey: "connected" },
  connecting: { icon: SpinnerGap, color: "var(--orange)", labelKey: "loading" },
  failed: { icon: WarningCircle, color: "var(--red)", labelKey: "error" },
  disabled: { icon: WifiSlash, color: "var(--fill-quaternary)", labelKey: "disable" },
};

const TRANSPORT_ICONS: Record<string, typeof Terminal> = {
  stdio: Terminal,
  sse: Globe,
  streamable_http: Lightning,
  http: Globe,
};

export function McpDetailModal({ open, pluginId, onClose, onEditConfig }: McpDetailModalProps) {
  const { t } = useTranslation("plugins");
  const restartPlugin = usePluginStore((s) => s.restartPlugin);
  const removePlugin = usePluginStore((s) => s.removePlugin);

  const [detail, setDetail] = useState<McpDetailResult | null>(null);
  const [loading, setLoading] = useState(false);
  const [restarting, setRestarting] = useState(false);
  const [confirmRemove, setConfirmRemove] = useState(false);
  const [removing, setRemoving] = useState(false);
  const [toolsOpen, setToolsOpen] = useState(true);
  const [toolSearch, setToolSearch] = useState("");
  const [prompts, setPrompts] = useState<McpPromptInfo[]>([]);
  const [promptsOpen, setPromptsOpen] = useState(true);
  const [resources, setResources] = useState<McpResourceInfo[]>([]);
  const [resourcesOpen, setResourcesOpen] = useState(true);

  const registryMap = useMemo(
    () => new Map<string, McpRegistryEntry>(registry.map((e) => [e.id, e])),
    [],
  );

  const plugins = usePluginStore((s) => s.plugins);
  const pluginCaps = useMemo(() => {
    const p = plugins.find((pl) => pl.id === pluginId);
    return p?.capabilities ?? { tools: true, resources: false, prompts: false };
  }, [plugins, pluginId]);

  useEffect(() => {
    if (!open || !pluginId) {
      setDetail(null);
      setConfirmRemove(false);
      setToolSearch("");
      setPrompts([]);
      setResources([]);
      return;
    }
    let cancelled = false;
    setLoading(true);
    transport.mcpDetail(pluginId)
      .then((d) => { if (!cancelled) { setDetail(d); setLoading(false); } })
      .catch(() => { if (!cancelled) { setDetail(null); setLoading(false); } });
    if (pluginCaps.prompts) {
      transport.mcpPrompts()
        .then((all) => { if (!cancelled) setPrompts(all.filter((p) => p.server === pluginId)); })
        .catch(() => {});
    }
    if (pluginCaps.resources) {
      transport.mcpResources(pluginId)
        .then((r) => { if (!cancelled) setResources(r); })
        .catch(() => {});
    }
    return () => { cancelled = true; };
  }, [open, pluginId, pluginCaps.prompts, pluginCaps.resources]);

  const handleRestart = useCallback(async () => {
    if (!pluginId || restarting) return;
    setRestarting(true);
    try {
      await restartPlugin(pluginId);
      const refreshed = await transport.mcpDetail(pluginId).catch(() => null);
      if (refreshed) setDetail(refreshed);
    } finally {
      setRestarting(false);
    }
  }, [pluginId, restarting, restartPlugin]);

  const handleRemove = useCallback(async () => {
    if (!pluginId || removing) return;
    setRemoving(true);
    const ok = await removePlugin(pluginId);
    setRemoving(false);
    if (ok) {
      onClose();
    } else {
      setConfirmRemove(false);
    }
  }, [pluginId, removing, removePlugin, onClose]);

  const filteredTools = useMemo(() => {
    if (!detail?.tools) return [];
    if (!toolSearch.trim()) return detail.tools;
    const q = toolSearch.toLowerCase();
    return detail.tools.filter(
      (tool) => tool.name.toLowerCase().includes(q) || (tool.description ?? "").toLowerCase().includes(q),
    );
  }, [detail?.tools, toolSearch]);

  if (!open || !pluginId) return null;

  const entry = registryMap.get(pluginId);
  const brandColor = entry?.brandColor ?? "var(--tint)";
  const IconComp = entry ? (ICON_MAP[entry.icon] ?? PuzzlePiece) : PuzzlePiece;
  const statusCfg = STATUS_CONFIG[detail?.status ?? "disabled"] ?? STATUS_CONFIG.disabled;
  const StatusIcon = statusCfg.icon;
  const TransportIcon = TRANSPORT_ICONS[detail?.config?.transport ?? "stdio"] ?? Terminal;

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center"
      style={{ background: "rgba(0,0,0,0.45)" }}
      onClick={(e) => { if (e.target === e.currentTarget) onClose(); }}
    >
      <div
        className="pv-modal-enter w-[520px] max-h-[85vh] flex flex-col rounded-xl overflow-hidden shadow-2xl"
        style={{ background: "var(--bg-card)", border: "0.5px solid var(--separator)" }}
      >
        {/* Gradient accent bar */}
        <div
          className="h-[3px] shrink-0"
          style={{ background: `linear-gradient(90deg, ${brandColor} 40%, transparent)` }}
        />

        {/* Hero header */}
        <div
          className="relative px-5 py-4 shrink-0"
          style={{ background: `color-mix(in srgb, ${brandColor} 5%, var(--bg-card))` }}
        >
          <button
            onClick={onClose}
            className="absolute right-3 top-3 flex items-center justify-center w-6 h-6 rounded-md transition-colors hover:bg-[var(--bg-hover)]"
            style={{ color: "var(--fill-tertiary)", background: "transparent", border: "none", cursor: "pointer" }}
          >
            <X size={ICON_SIZE.sm} />
          </button>
          <div className="flex items-start gap-3.5">
            <div
              className="flex h-12 w-12 shrink-0 items-center justify-center rounded-[12px]"
              style={{ background: `color-mix(in srgb, ${brandColor} 12%, transparent)` }}
            >
              <IconComp size={24} style={{ color: brandColor }} />
            </div>
            <div className="min-w-0 flex-1 pr-6">
              <h2 className="truncate text-[18px] font-semibold leading-tight" style={{ color: "var(--fill-primary)" }}>
                {entry?.name ?? detail?.id ?? pluginId}
              </h2>
              {entry?.author && (
                <span className="text-[11px]" style={{ color: "var(--fill-quaternary)" }}>
                  {t("detail.by_author", { author: entry.author })}
                </span>
              )}
              {entry?.description && (
                <p className="mt-1 text-[12px] leading-relaxed" style={{ color: "var(--fill-tertiary)" }}>
                  {entry.description}
                </p>
              )}
              <div className="mt-2 flex items-center gap-2">
                <span
                  className="flex items-center gap-1 shrink-0 rounded-full px-2 py-0.5 text-[10px] font-medium"
                  style={{ color: statusCfg.color, background: `color-mix(in srgb, ${statusCfg.color} 10%, transparent)` }}
                >
                  <StatusIcon size={ICON_SIZE.xs} className={detail?.status === "connecting" ? "animate-spin" : ""} />
                  {t(statusCfg.labelKey)}
                </span>
                {detail && detail.tools.length > 0 && (
                  <span className="text-[10px]" style={{ color: "var(--fill-quaternary)" }}>
                    {t("tools_count", { count: detail.tools.length })}
                  </span>
                )}
              </div>
            </div>
          </div>
        </div>

        {/* Body */}
        <div className="flex-1 overflow-y-auto px-5 py-4 flex flex-col gap-5">
          {loading ? (
            <div className="flex items-center justify-center py-12">
              <SpinnerGap size={ICON_SIZE.lg} className="animate-spin" style={{ color: "var(--fill-quaternary)" }} />
            </div>
          ) : detail ? (
            <>
              {/* Config Section */}
              <Section title={t("detail.config_title")}>
                <div className="flex flex-col gap-2">
                  <ConfigRow label={t("detail.transport")} icon={<TransportIcon size={ICON_SIZE.xs} />}>
                    {detail.config.transport}
                  </ConfigRow>
                  {detail.config.command && (
                    <ConfigRow label={t("detail.command")}>
                      <code className="text-[12px]" style={{ fontFamily: "var(--font-mono, monospace)" }}>
                        {detail.config.command} {detail.config.args.join(" ")}
                      </code>
                    </ConfigRow>
                  )}
                  {detail.config.url && (
                    <ConfigRow label={t("detail.url")}>
                      <code className="text-[12px]" style={{ fontFamily: "var(--font-mono, monospace)" }}>
                        {detail.config.url}
                      </code>
                    </ConfigRow>
                  )}
                  {detail.config.source && (
                    <ConfigRow label={t("detail.source")}>
                      {detail.config.source}
                    </ConfigRow>
                  )}
                  {detail.connectedAt && (
                    <ConfigRow label={t("detail.connected_at")} icon={<Clock size={ICON_SIZE.xs} />}>
                      {new Date(detail.connectedAt).toLocaleString()}
                    </ConfigRow>
                  )}
                </div>
                {onEditConfig && (
                  <button
                    onClick={() => onEditConfig(pluginId!)}
                    className="mt-2.5 flex items-center gap-1 text-[11px] font-medium transition-colors hover:opacity-80"
                    style={{ color: "var(--tint)", background: "none", border: "none", cursor: "pointer", padding: 0 }}
                  >
                    <PencilSimple size={ICON_SIZE.xs} />
                    {t("detail.edit_config")}
                  </button>
                )}
              </Section>

              {/* Env Section */}
              {detail.config.env && Object.keys(detail.config.env).length > 0 && (
                <Section title={t("detail.env_title")}>
                  <div className="flex flex-col gap-1">
                    {Object.entries(detail.config.env).map(([k, v]) => (
                      <div key={k} className="flex items-baseline gap-2 text-[12px]" style={{ fontFamily: "var(--font-mono, monospace)" }}>
                        <span style={{ color: "var(--fill-secondary)" }}>{k}</span>
                        <span style={{ color: "var(--fill-quaternary)" }}>=</span>
                        <span style={{ color: "var(--fill-tertiary)" }}>{maskEnvValue(v)}</span>
                      </div>
                    ))}
                  </div>
                </Section>
              )}

              {/* Error */}
              {detail.error && (
                <div className="rounded-lg px-3.5 py-2.5 text-[12px]" style={{ color: "var(--red)", background: "rgba(239,68,68,0.06)", border: "0.5px solid rgba(239,68,68,0.15)" }}>
                  <div className="flex items-center gap-1.5 mb-1 font-semibold">
                    <WarningCircle size={ICON_SIZE.sm} />
                    {t("detail.error_title")}
                  </div>
                  <p style={{ fontFamily: "var(--font-mono, monospace)", wordBreak: "break-all" }}>{detail.error}</p>
                </div>
              )}

              {/* Tools Section — collapsible + searchable */}
              <div>
                <button
                  onClick={() => setToolsOpen((o) => !o)}
                  className="flex w-full items-center gap-1.5 mb-2"
                  style={{ color: "var(--fill-quaternary)", background: "none", border: "none", cursor: "pointer", padding: 0 }}
                >
                  {toolsOpen ? <CaretDown size={ICON_SIZE.xs} /> : <CaretRight size={ICON_SIZE.xs} />}
                  <h3 className="text-[11px] font-semibold uppercase tracking-wider">
                    {t("detail.tools_title")} ({detail.tools.length})
                  </h3>
                </button>
                {toolsOpen && (
                  <>
                    {detail.tools.length > 5 && (
                      <div className="relative mb-2">
                        <MagnifyingGlass
                          size={12}
                          className="absolute left-2.5 top-1/2 -translate-y-1/2"
                          style={{ color: "var(--fill-quaternary)" }}
                        />
                        <input
                          type="text"
                          value={toolSearch}
                          onChange={(e) => setToolSearch(e.target.value)}
                          placeholder={t("detail.search_tools")}
                          className="w-full rounded-md py-1.5 pl-7 pr-2 text-[11px] outline-none"
                          style={{ background: "var(--bg-tertiary)", border: "0.5px solid var(--separator)", color: "var(--fill-primary)" }}
                        />
                      </div>
                    )}
                    {filteredTools.length === 0 ? (
                      <p className="text-[12px]" style={{ color: "var(--fill-quaternary)" }}>
                        {t("no_tools")}
                      </p>
                    ) : (
                      <div className="flex flex-col gap-1.5">
                        {filteredTools.map((tool) => (
                          <div
                            key={tool.name}
                            className="flex items-start gap-2 rounded-md px-2.5 py-2"
                            style={{ background: "var(--bg-tertiary)" }}
                          >
                            <Wrench size={ICON_SIZE.xs} className="mt-0.5 shrink-0" style={{ color: "var(--fill-quaternary)" }} />
                            <div className="min-w-0">
                              <span className="text-[12px] font-semibold" style={{ color: "var(--fill-primary)", fontFamily: "var(--font-mono, monospace)" }}>
                                {tool.name}
                              </span>
                              {tool.description && (
                                <p className="mt-0.5 text-[11px] leading-relaxed" style={{ color: "var(--fill-tertiary)" }}>
                                  {tool.description}
                                </p>
                              )}
                            </div>
                          </div>
                        ))}
                      </div>
                    )}
                  </>
                )}
              </div>

              {/* Resources Section */}
              {pluginCaps.resources && resources.length > 0 && (
                <div>
                  <button
                    onClick={() => setResourcesOpen((o) => !o)}
                    className="flex w-full items-center gap-1.5 mb-2"
                    style={{ color: "var(--fill-quaternary)", background: "none", border: "none", cursor: "pointer", padding: 0 }}
                  >
                    {resourcesOpen ? <CaretDown size={ICON_SIZE.xs} /> : <CaretRight size={ICON_SIZE.xs} />}
                    <h3 className="text-[11px] font-semibold uppercase tracking-wider">
                      {t("detail.resources_title")} ({resources.length})
                    </h3>
                  </button>
                  {resourcesOpen && (
                    <div className="flex flex-col gap-1.5">
                      {resources.map((r) => (
                        <div
                          key={r.uri}
                          className="flex items-start gap-2 rounded-md px-2.5 py-2"
                          style={{ background: "var(--bg-tertiary)" }}
                        >
                          <Files size={ICON_SIZE.xs} className="mt-0.5 shrink-0" style={{ color: "var(--fill-quaternary)" }} />
                          <div className="min-w-0">
                            <span className="text-[12px] font-semibold" style={{ color: "var(--fill-primary)" }}>
                              {r.name}
                            </span>
                            <p className="text-[10px]" style={{ color: "var(--fill-quaternary)", fontFamily: "var(--font-mono, monospace)", wordBreak: "break-all" }}>
                              {r.uri}
                            </p>
                            {r.description && (
                              <p className="mt-0.5 text-[11px] leading-relaxed" style={{ color: "var(--fill-tertiary)" }}>
                                {r.description}
                              </p>
                            )}
                          </div>
                        </div>
                      ))}
                    </div>
                  )}
                </div>
              )}

              {/* Prompts Section */}
              {pluginCaps.prompts && prompts.length > 0 && (
                <div>
                  <button
                    onClick={() => setPromptsOpen((o) => !o)}
                    className="flex w-full items-center gap-1.5 mb-2"
                    style={{ color: "var(--fill-quaternary)", background: "none", border: "none", cursor: "pointer", padding: 0 }}
                  >
                    {promptsOpen ? <CaretDown size={ICON_SIZE.xs} /> : <CaretRight size={ICON_SIZE.xs} />}
                    <h3 className="text-[11px] font-semibold uppercase tracking-wider">
                      {t("detail.prompts_title")} ({prompts.length})
                    </h3>
                  </button>
                  {promptsOpen && (
                    <div className="flex flex-col gap-1.5">
                      {prompts.map((p) => (
                        <div
                          key={p.name}
                          className="flex items-start gap-2 rounded-md px-2.5 py-2"
                          style={{ background: "var(--bg-tertiary)" }}
                        >
                          <ChatText size={ICON_SIZE.xs} className="mt-0.5 shrink-0" style={{ color: "var(--fill-quaternary)" }} />
                          <div className="min-w-0">
                            <span className="text-[12px] font-semibold" style={{ color: "var(--fill-primary)", fontFamily: "var(--font-mono, monospace)" }}>
                              {p.name}
                            </span>
                            {p.description && (
                              <p className="mt-0.5 text-[11px] leading-relaxed" style={{ color: "var(--fill-tertiary)" }}>
                                {p.description}
                              </p>
                            )}
                            {p.arguments && p.arguments.length > 0 && (
                              <div className="mt-1 flex flex-wrap gap-1">
                                {p.arguments.map((arg) => (
                                  <span
                                    key={arg.name}
                                    className="rounded px-1.5 py-0.5 text-[10px]"
                                    style={{
                                      background: "var(--bg-secondary)",
                                      color: arg.required ? "var(--fill-primary)" : "var(--fill-tertiary)",
                                      border: "0.5px solid var(--separator)",
                                    }}
                                    title={arg.description}
                                  >
                                    {arg.name}{arg.required ? "*" : ""}
                                  </span>
                                ))}
                              </div>
                            )}
                          </div>
                        </div>
                      ))}
                    </div>
                  )}
                </div>
              )}
            </>
          ) : (
            <p className="py-8 text-center text-[13px]" style={{ color: "var(--fill-quaternary)" }}>
              {t("failed_load_details")}
            </p>
          )}
        </div>

        {/* Footer */}
        <div
          className="flex items-center justify-between px-5 py-3 shrink-0"
          style={{ borderTop: "0.5px solid var(--separator)" }}
        >
          <div>
            {confirmRemove ? (
              <div className="flex items-center gap-2">
                <span className="text-[12px]" style={{ color: "var(--red)" }}>{t("remove_confirm")}</span>
                <button
                  onClick={handleRemove}
                  disabled={removing}
                  className="rounded-md px-2.5 py-1 text-[11px] font-medium"
                  style={{ cursor: "pointer", background: "var(--red)", color: "#fff", border: "none", opacity: removing ? 0.6 : 1 }}
                >
                  {removing ? t("detail.removing") : t("remove_yes")}
                </button>
                <button
                  onClick={() => setConfirmRemove(false)}
                  className="rounded-md px-2.5 py-1 text-[11px] font-medium"
                  style={{ cursor: "pointer", background: "var(--bg-tertiary)", color: "var(--fill-secondary)", border: "none" }}
                >
                  {t("remove_no")}
                </button>
              </div>
            ) : (
              <button
                onClick={() => setConfirmRemove(true)}
                className="flex items-center gap-1 rounded-md px-2.5 py-1.5 text-[12px] font-medium transition-colors hover:bg-[rgba(239,68,68,0.08)]"
                style={{ color: "var(--red)", background: "transparent", border: "none", cursor: "pointer" }}
              >
                <Trash size={ICON_SIZE.xs} />
                {t("detail.remove")}
              </button>
            )}
          </div>
          <button
            onClick={handleRestart}
            disabled={restarting}
            className={BTN_PRIMARY_SM}
            style={{ opacity: restarting ? 0.6 : 1 }}
          >
            <ArrowsClockwise size={ICON_SIZE.xs} className={restarting ? "animate-spin" : ""} />
            {restarting ? t("detail.restarting") : t("detail.restart")}
          </button>
        </div>
      </div>
    </div>
  );
}

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div>
      <h3 className="mb-2 text-[11px] font-semibold uppercase tracking-wider" style={{ color: "var(--fill-quaternary)" }}>
        {title}
      </h3>
      {children}
    </div>
  );
}

function ConfigRow({ label, icon, children }: { label: string; icon?: React.ReactNode; children: React.ReactNode }) {
  return (
    <div className="flex items-baseline gap-3">
      <span className="flex items-center gap-1 shrink-0 text-[11px]" style={{ color: "var(--fill-quaternary)", minWidth: 80 }}>
        {icon}
        {label}
      </span>
      <span className="text-[12px]" style={{ color: "var(--fill-secondary)", wordBreak: "break-all" }}>
        {children}
      </span>
    </div>
  );
}

function maskEnvValue(v: string): string {
  if (!v || v.length <= 4) return v || '""';
  return `${v.slice(0, 3)}${"•".repeat(Math.min(v.length - 3, 12))}`;
}
