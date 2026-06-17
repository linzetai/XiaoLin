import { useEffect, useState, useCallback, useMemo, useRef } from "react";
import type { Icon } from "@phosphor-icons/react";
import {
  PuzzlePiece, ToggleLeft, ToggleRight, ArrowsClockwise,
  CaretDown, CaretRight, WarningCircle, SpinnerGap,
  Wrench, Globe, User, UploadSimple, FolderOpen, FileText,
  WifiHigh, WifiSlash, Link, LinkBreak,
  QrCode, CheckCircle, DeviceMobile, Key, Terminal,
  PencilSimple, ArrowCounterClockwise, FloppyDisk,
  MagnifyingGlass, CaretUp, X,
  ShieldWarning, CheckFat, XCircle,
  Plus, Trash,
} from "@phosphor-icons/react";
import { useTranslation } from "react-i18next";
import { usePluginStore, subscribePluginEvents } from "../../lib/stores/plugin-store";
import { useGatewayStore } from "../../lib/store";
import type { PluginSummary, PluginTool, ChannelStatus, ChannelDetailResult } from "../../lib/transport";
import * as api from "../../lib/api";
import { ICON_SIZE, BTN_TEXT_SM } from "../../lib/ui-tokens";
import { SegmentedControl } from "../common/SegmentedControl";
import { AddServerModal } from "./AddServerModal";
import { McpExplorePanel, registry as mcpRegistry, MCP_ICON_MAP } from "./McpExplorePanel";
import type { McpRegistryEntry } from "./McpExplorePanel";
import { McpDetailModal } from "./McpDetailModal";

type PluginsTab = "mcp" | "skills" | "channels";
type McpSubView = "installed" | "explore";

const PLUGIN_ICON_MAP: Record<string, Icon> = {
  ...MCP_ICON_MAP,
  PuzzlePiece,
};

export function PluginsView() {
  const { t } = useTranslation("plugins");
  const [activeTab, setActiveTab] = useState<PluginsTab>("mcp");
  const [skillCount, setSkillCount] = useState(0);
  const [channelCount, setChannelCount] = useState(0);
  const [addModalOpen, setAddModalOpen] = useState(false);
  const [addModalPrefill, setAddModalPrefill] = useState<{ id?: string; command?: string; args?: string[]; transport?: "stdio" | "sse" | "streamable_http"; url?: string } | undefined>(undefined);
  const [detailPluginId, setDetailPluginId] = useState<string | null>(null);

  const plugins = usePluginStore((s) => s.plugins);
  const mcpCount = plugins.length;

  const handleEditConfig = useCallback(async (pluginId: string) => {
    setDetailPluginId(null);
    try {
      const detail = await api.mcpDetail(pluginId);
      if (detail) {
        setAddModalPrefill({
          id: detail.id,
          transport: (detail.config.transport as "stdio" | "sse" | "streamable_http") ?? "stdio",
          ...(detail.config.command ? { command: detail.config.command } : {}),
          ...(detail.config.args?.length ? { args: detail.config.args } : {}),
          ...(detail.config.url ? { url: detail.config.url } : {}),
        });
      } else {
        setAddModalPrefill({ id: pluginId });
      }
    } catch {
      setAddModalPrefill({ id: pluginId });
    }
    setAddModalOpen(true);
  }, []);

  const handleCloseAddModal = useCallback(() => {
    setAddModalOpen(false);
    setAddModalPrefill(undefined);
  }, []);

  const tabItems = useMemo(() => [
    { value: "mcp" as const, label: t("tab_mcp"), count: mcpCount },
    { value: "skills" as const, label: t("tab_skills"), count: skillCount },
    { value: "channels" as const, label: t("tab_channels"), count: channelCount },
  ], [t, mcpCount, skillCount, channelCount]);

  return (
    <div className="flex h-full flex-col" style={{ background: "var(--bg-card)" }}>
      <div
        className="flex shrink-0 flex-col gap-3 px-6 py-4"
        style={{ borderBottom: "0.5px solid var(--separator)" }}
      >
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-2.5">
            <PuzzlePiece size={ICON_SIZE.md} style={{ color: "var(--fill-tertiary)" }} />
            <h1 className="text-[16px] font-semibold tracking-[-0.01em]" style={{ color: "var(--fill-primary)" }}>
              {t("title")}
            </h1>
          </div>
          <TabActions activeTab={activeTab} onAddServer={() => { setAddModalPrefill(undefined); setAddModalOpen(true); }} />
        </div>

        <SegmentedControl value={activeTab} onChange={setActiveTab} items={tabItems} />
      </div>

      <div className="flex-1 overflow-y-auto" style={{ overscrollBehavior: "contain" }}>
        <div key={activeTab} className="pv-fade-in">
          {activeTab === "mcp" && <McpTabContent onDetail={setDetailPluginId} onAdd={() => setAddModalOpen(true)} />}
          {activeTab === "skills" && <SkillsTabContent onCountChange={setSkillCount} />}
          {activeTab === "channels" && <ChannelsTabContent onCountChange={setChannelCount} />}
        </div>
      </div>

      <AddServerModal open={addModalOpen} onClose={handleCloseAddModal} prefill={addModalPrefill} />
      <McpDetailModal open={detailPluginId !== null} pluginId={detailPluginId} onClose={() => setDetailPluginId(null)} onEditConfig={handleEditConfig} />
    </div>
  );
}

// ─── Tab Action Buttons ───

function TabActions({ activeTab, onAddServer }: { activeTab: PluginsTab; onAddServer: () => void }) {
  const { t } = useTranslation("plugins");
  const fetchPlugins = usePluginStore((s) => s.fetchPlugins);

  if (activeTab === "mcp") {
    return (
      <div className="flex items-center gap-1.5">
        <button onClick={onAddServer} className={BTN_TEXT_SM}>
          <Plus size={ICON_SIZE.xs} weight="bold" /> {t("add_server")}
        </button>
        <button onClick={() => fetchPlugins()} className={BTN_TEXT_SM}>
          <ArrowsClockwise size={ICON_SIZE.xs} /> {t("reload")}
        </button>
      </div>
    );
  }
  return null;
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// MCP Tab
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

function McpTabContent({ onDetail, onAdd }: { onDetail: (id: string) => void; onAdd: () => void }) {
  const { t } = useTranslation("plugins");
  const plugins = usePluginStore((s) => s.plugins);
  const loading = usePluginStore((s) => s.loading);
  const fetchPlugins = usePluginStore((s) => s.fetchPlugins);
  const enablePlugin = usePluginStore((s) => s.enablePlugin);
  const disablePlugin = usePluginStore((s) => s.disablePlugin);
  const restartPlugin = usePluginStore((s) => s.restartPlugin);
  const removePlugin = usePluginStore((s) => s.removePlugin);
  const approvePlugin = usePluginStore((s) => s.approvePlugin);
  const rejectPlugin = usePluginStore((s) => s.rejectPlugin);
  const oauthLoginPlugin = usePluginStore((s) => s.oauthLoginPlugin);
  const fetchTools = usePluginStore((s) => s.fetchTools);
  const toolsById = usePluginStore((s) => s.toolsById);

  const [mcpSubView, setMcpSubView] = useState<McpSubView>("installed");
  const [expandedId, setExpandedId] = useState<string | null>(null);

  useEffect(() => {
    subscribePluginEvents();
    fetchPlugins();
  }, [fetchPlugins]);

  const handleToggle = useCallback(
    async (p: PluginSummary) => {
      if (p.enabled) await disablePlugin(p.id);
      else await enablePlugin(p.id);
    },
    [enablePlugin, disablePlugin],
  );

  const handleRestart = useCallback(
    async (id: string) => { await restartPlugin(id); },
    [restartPlugin],
  );

  const handleExpand = useCallback(
    (id: string) => {
      const next = expandedId === id ? null : id;
      setExpandedId(next);
      if (next && !toolsById[next]) fetchTools(next);
    },
    [expandedId, toolsById, fetchTools],
  );

  const handleRemove = useCallback(
    async (id: string) => removePlugin(id),
    [removePlugin],
  );

  const handleOauthLogin = useCallback(
    async (id: string) => { await oauthLoginPlugin(id); },
    [oauthLoginPlugin],
  );

  const pendingPlugins = useMemo(
    () => plugins.filter((p) => p.status === "pending_approval"),
    [plugins],
  );
  const activePlugins = useMemo(
    () => plugins.filter((p) => p.status !== "pending_approval"),
    [plugins],
  );
  const connectedCount = activePlugins.filter((p) => p.status === "connected").length;
  const userPlugins = useMemo(
    () => activePlugins.filter((p) => p.scope !== "project"),
    [activePlugins],
  );
  const projectPlugins = useMemo(
    () => activePlugins.filter((p) => p.scope === "project"),
    [activePlugins],
  );

  const registryMap = useMemo(
    () => new Map<string, McpRegistryEntry>(mcpRegistry.map((e) => [e.id, e])),
    [],
  );

  const subViewItems = useMemo(() => [
    { value: "installed" as const, label: t("mcp_sub.installed"), count: plugins.length },
    { value: "explore" as const, label: t("mcp_sub.explore") },
  ], [t, plugins.length]);

  return (
    <div>
      {/* Sub-view toggle */}
      <div className="mx-auto w-full px-6 pt-4" style={{ maxWidth: "var(--content-max-w)" }}>
        <div className="flex gap-1 rounded-lg p-0.5" style={{ background: "var(--bg-tertiary)" }}>
          {subViewItems.map((item) => {
            const active = mcpSubView === item.value;
            return (
              <button
                key={item.value}
                onClick={() => setMcpSubView(item.value)}
                className="flex-1 rounded-md px-3 py-1.5 text-[12px] font-medium transition-colors"
                style={{
                  background: active ? "var(--bg-card)" : "transparent",
                  color: active ? "var(--fill-primary)" : "var(--fill-tertiary)",
                  border: "none",
                  cursor: "pointer",
                  boxShadow: active ? "0 1px 2px rgba(0,0,0,0.06)" : "none",
                }}
              >
                {item.label}
                {"count" in item && item.count != null && (
                  <span className="ml-1 opacity-60">{item.count}</span>
                )}
              </button>
            );
          })}
        </div>
      </div>

      {mcpSubView === "explore" ? (
        <McpExplorePanel />
      ) : loading ? (
        <div className="flex flex-col items-center justify-center gap-3 py-20 pv-fade-in">
          <SpinnerGap size={ICON_SIZE.lg} className="animate-spin" style={{ color: "var(--fill-quaternary)" }} />
          <p className="text-xs" style={{ color: "var(--fill-quaternary)" }}>{t("loading_plugins")}</p>
        </div>
      ) : plugins.length === 0 ? (
        <McpEmptyState onExplore={() => setMcpSubView("explore")} onAdd={onAdd} />
      ) : (
        <div className="mx-auto w-full px-6 py-5" style={{ maxWidth: "var(--content-max-w)" }}>
          {pendingPlugins.length > 0 && (
            <PendingApprovalSection
              plugins={pendingPlugins}
              onApprove={approvePlugin}
              onReject={rejectPlugin}
            />
          )}
          {connectedCount > 0 && (
            <div className="mb-3 flex items-center gap-1.5">
              <span className="inline-block h-1.5 w-1.5 rounded-full" style={{ background: "var(--green)" }} />
              <span className="text-xs font-semibold tabular-nums" style={{ color: "var(--green)" }}>
                {t("connected_count", { connected: connectedCount, total: activePlugins.length })}
              </span>
            </div>
          )}
          {userPlugins.length > 0 && (
            <PluginGroup
              label={t("group.user")}
              plugins={userPlugins}
              expandedId={expandedId}
              toolsById={toolsById}
              registryMap={registryMap}
              onToggle={handleToggle}
              onRestart={handleRestart}
              onExpand={handleExpand}
              onRemove={handleRemove}
              onDetail={onDetail}
              onOauthLogin={handleOauthLogin}
            />
          )}
          {projectPlugins.length > 0 && (
            <PluginGroup
              label={t("group.project")}
              plugins={projectPlugins}
              expandedId={expandedId}
              toolsById={toolsById}
              registryMap={registryMap}
              onToggle={handleToggle}
              onRestart={handleRestart}
              onExpand={handleExpand}
              onRemove={handleRemove}
              onDetail={onDetail}
              onOauthLogin={handleOauthLogin}
              className={userPlugins.length > 0 ? "mt-4" : undefined}
            />
          )}
        </div>
      )}
    </div>
  );
}

function PluginGroup({
  label, plugins, expandedId, toolsById, registryMap, onToggle, onRestart, onExpand, onRemove, onDetail, onOauthLogin, className,
}: {
  label: string;
  plugins: PluginSummary[];
  expandedId: string | null;
  toolsById: Record<string, PluginTool[] | undefined>;
  registryMap: Map<string, McpRegistryEntry>;
  onToggle: (p: PluginSummary) => void;
  onRestart: (id: string) => void;
  onExpand: (id: string) => void;
  onRemove: (id: string) => Promise<boolean>;
  onDetail: (id: string) => void;
  onOauthLogin: (id: string) => void;
  className?: string;
}) {
  return (
    <div className={className}>
      <div className="mb-2 flex items-center gap-1.5">
        <span className="text-[11px] font-semibold uppercase tracking-wider" style={{ color: "var(--fill-quaternary)" }}>
          {label}
        </span>
        <span className="text-[10px] tabular-nums" style={{ color: "var(--fill-quaternary)" }}>({plugins.length})</span>
      </div>
      <div className="flex flex-col gap-2">
        {plugins.map((p, idx) => (
          <PluginRow
            key={p.id}
            plugin={p}
            expanded={expandedId === p.id}
            tools={toolsById[p.id]}
            registryEntry={registryMap.get(p.id)}
            onToggle={() => onToggle(p)}
            onRestart={() => onRestart(p.id)}
            onExpand={() => onExpand(p.id)}
            onRemove={() => onRemove(p.id)}
            onDetail={() => onDetail(p.id)}
            onOauthLogin={() => onOauthLogin(p.id)}
            index={idx}
          />
        ))}
      </div>
    </div>
  );
}

function PendingApprovalSection({
  plugins,
  onApprove,
  onReject,
}: {
  plugins: PluginSummary[];
  onApprove: (id: string) => Promise<boolean>;
  onReject: (id: string) => Promise<boolean>;
}) {
  return (
    <div
      className="mb-5 overflow-hidden rounded-[var(--radius-sm)] pv-fade-in"
      style={{
        background: "color-mix(in srgb, var(--orange, #ED8936) 4%, var(--bg-primary))",
        border: "0.5px solid color-mix(in srgb, var(--orange, #ED8936) 20%, transparent)",
      }}
    >
      <div
        className="flex items-center gap-2 px-4 py-2.5"
        style={{ borderBottom: "0.5px solid color-mix(in srgb, var(--orange, #ED8936) 12%, transparent)" }}
      >
        <ShieldWarning size={ICON_SIZE.sm} weight="fill" style={{ color: "var(--orange, #ED8936)" }} />
        <span className="text-[12px] font-semibold" style={{ color: "var(--orange, #ED8936)" }}>
          Project MCP Servers Need Approval
        </span>
      </div>
      <div className="flex flex-col">
        {plugins.map((p, idx) => (
          <PendingApprovalCard
            key={p.id}
            plugin={p}
            onApprove={onApprove}
            onReject={onReject}
            isLast={idx === plugins.length - 1}
          />
        ))}
      </div>
    </div>
  );
}

function PendingApprovalCard({
  plugin,
  onApprove,
  onReject,
  isLast,
}: {
  plugin: PluginSummary;
  onApprove: (id: string) => Promise<boolean>;
  onReject: (id: string) => Promise<boolean>;
  isLast: boolean;
}) {
  const [acting, setActing] = useState<"approve" | "reject" | null>(null);
  const mountedRef = useRef(true);
  useEffect(() => () => { mountedRef.current = false; }, []);

  const handleApprove = async () => {
    setActing("approve");
    await onApprove(plugin.id);
    if (mountedRef.current) setActing(null);
  };

  const handleReject = async () => {
    setActing("reject");
    await onReject(plugin.id);
    if (mountedRef.current) setActing(null);
  };

  return (
    <div
      className="flex items-center gap-3 px-4 py-3"
      style={!isLast ? { borderBottom: "0.5px solid color-mix(in srgb, var(--orange, #ED8936) 10%, transparent)" } : undefined}
    >
      <StatusDot status="pending_approval" />
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2">
          <span className="text-[13px] font-semibold" style={{ color: "var(--fill-primary)" }}>
            {plugin.name}
          </span>
          <ScopeBadge scope={plugin.scope} />
        </div>
        {plugin.commandPreview && (
          <div
            className="mt-1 inline-flex items-center gap-1.5 rounded px-2 py-1 font-mono text-[11px]"
            style={{
              background: "var(--bg-tertiary)",
              color: "var(--fill-tertiary)",
            }}
          >
            <Terminal size={ICON_SIZE.xs} />
            <span className="truncate" style={{ maxWidth: 280 }}>{plugin.commandPreview}</span>
          </div>
        )}
      </div>
      <div className="flex shrink-0 items-center gap-1.5">
        <button
          onClick={handleApprove}
          disabled={acting !== null}
          className="flex items-center gap-1 rounded-md px-2.5 py-1.5 text-[11px] font-semibold transition-all hover:brightness-110 disabled:opacity-50"
          style={{
            cursor: acting ? "wait" : "pointer",
            background: "var(--green, #38A169)",
            border: "none",
            color: "#fff",
          }}
        >
          {acting === "approve" ? (
            <SpinnerGap size={ICON_SIZE.xs} className="animate-spin" />
          ) : (
            <CheckFat size={ICON_SIZE.xs} weight="fill" />
          )}
          Approve
        </button>
        <button
          onClick={handleReject}
          disabled={acting !== null}
          className="flex items-center gap-1 rounded-md px-2.5 py-1.5 text-[11px] font-semibold transition-all hover:bg-[var(--bg-hover)] disabled:opacity-50"
          style={{
            cursor: acting ? "wait" : "pointer",
            background: "none",
            border: "0.5px solid var(--separator)",
            color: "var(--fill-tertiary)",
          }}
        >
          {acting === "reject" ? (
            <SpinnerGap size={ICON_SIZE.xs} className="animate-spin" />
          ) : (
            <XCircle size={ICON_SIZE.xs} />
          )}
          Reject
        </button>
      </div>
    </div>
  );
}

function McpEmptyState({ onExplore, onAdd }: { onExplore?: () => void; onAdd?: () => void }) {
  const { t } = useTranslation("plugins");
  return (
    <div className="flex flex-col items-center justify-center gap-5 py-24 pv-fade-in">
      <div
        className="pv-float flex h-16 w-16 items-center justify-center rounded-[var(--radius-md)]"
        style={{ background: "color-mix(in srgb, var(--tint) 6%, transparent)" }}
      >
        <PuzzlePiece size={ICON_SIZE["2xl"]} style={{ color: "var(--tint)", opacity: 0.8 }} />
      </div>
      <div className="text-center">
        <p className="text-[16px] font-semibold" style={{ color: "var(--fill-primary)" }}>{t("no_mcp_title")}</p>
        <p className="mt-2 text-[13px] leading-relaxed" style={{ color: "var(--fill-quaternary)", maxWidth: 320 }}>
          {t("no_mcp_desc")}
        </p>
      </div>
      <div className="flex items-center gap-2">
        {onExplore && (
          <button
            onClick={onExplore}
            className="rounded-lg px-4 py-2 text-[13px] font-semibold transition-colors hover:opacity-90"
            style={{ background: "var(--tint)", color: "#fff", border: "none", cursor: "pointer" }}
          >
            {t("explore.browse_servers")}
          </button>
        )}
        {onAdd && (
          <button
            onClick={onAdd}
            className="rounded-lg px-4 py-2 text-[13px] font-semibold transition-colors hover:opacity-90"
            style={{ background: "transparent", color: "var(--fill-secondary)", border: "0.5px solid var(--separator)", cursor: "pointer" }}
          >
            {t("empty.add_manually")}
          </button>
        )}
      </div>
    </div>
  );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Skills Tab
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

function SkillsTabContent({ onCountChange }: { onCountChange: (n: number) => void }) {
  const { t } = useTranslation("plugins");
  const gatewayReady = useGatewayStore((s) => s.connected);
  const [publicSkills, setPublicSkills] = useState<api.SkillInfo[]>([]);
  const [agentSkillsMap, setAgentSkillsMap] = useState<Record<string, api.SkillInfo[]>>({});
  const [tools, setTools] = useState<api.ToolInfo[]>([]);
  const [loading, setLoading] = useState(true);
  const [filter, setFilter] = useState<"skills" | "tools">("skills");
  const [refreshing, setRefreshing] = useState(false);
  const [uploading, setUploading] = useState(false);
  const [skillMenuOpen, setSkillMenuOpen] = useState(false);

  const loadAllSkills = useCallback(async () => {
    try {
      const [globalSkills, mainSkills] = await Promise.all([
        api.listSkills(),
        api.listSkills("main"),
      ]);
      setPublicSkills(globalSkills);
      setAgentSkillsMap(mainSkills.length > 0 ? { main: mainSkills } : {});
    } catch { /* silent */ }
  }, []);

  useEffect(() => {
    if (!gatewayReady) return;
    const loadAll = async () => {
      const [, toolList] = await Promise.all([
        loadAllSkills(),
        api.listTools().catch(() => null),
      ]);
      if (toolList) setTools(toolList);
      setLoading(false);
    };
    loadAll();
  }, [gatewayReady, loadAllSkills]);

  const totalSkills = useMemo(
    () => publicSkills.length + Object.values(agentSkillsMap).reduce((s, a) => s + a.length, 0),
    [publicSkills, agentSkillsMap],
  );

  useEffect(() => {
    onCountChange(filter === "skills" ? totalSkills : tools.length);
  }, [totalSkills, tools.length, filter, onCountChange]);

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
      const selected = await open({ title: t("selectSkillFolder"), directory: true, multiple: false });
      if (selected) {
        await api.uploadSkill(selected as string);
        await api.refreshSkills();
        await loadAllSkills();
      }
    } catch { /* cancelled */ }
    setUploading(false);
  }, [loadAllSkills, t]);

  const handleUploadZip = useCallback(async () => {
    setUploading(true);
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const selected = await open({ title: t("selectSkillZip"), directory: false, multiple: false, filters: [{ name: "ZIP", extensions: ["zip"] }] });
      if (selected) {
        await api.uploadSkill(selected as string);
        await api.refreshSkills();
        await loadAllSkills();
      }
    } catch { /* cancelled */ }
    setUploading(false);
  }, [loadAllSkills, t]);

  if (loading) {
    return (
      <div className="flex flex-col items-center justify-center gap-3 py-20 pv-fade-in">
        <SpinnerGap size={ICON_SIZE.lg} className="animate-spin" style={{ color: "var(--fill-quaternary)" }} />
        <p className="text-xs" style={{ color: "var(--fill-quaternary)" }}>{t("loading_skills")}</p>
      </div>
    );
  }

  return (
    <div className="mx-auto w-full max-w-[var(--content-max-w)] px-6 py-5 pv-fade-in">
      {/* Sub-header: filter toggle + actions */}
      <div className="mb-4 flex items-center justify-between">
        <div className="flex rounded-md p-0.5" style={{ background: "var(--bg-tertiary)" }}>
          {(["skills", "tools"] as const).map((f) => (
            <button
              key={f}
              onClick={() => setFilter(f)}
              className="rounded-md px-2.5 py-1 text-[11px] font-medium transition-all duration-150"
              style={{
                background: filter === f ? "var(--bg-elevated)" : "transparent",
                color: filter === f ? "var(--fill-primary)" : "var(--fill-tertiary)",
                boxShadow: filter === f ? "var(--shadow-sm)" : "none",
                cursor: "pointer",
                border: "none",
              }}
            >
              {f === "skills" ? `Skills (${totalSkills})` : `Tools (${tools.length})`}
            </button>
          ))}
        </div>

        {filter === "skills" && (
          <div className="flex items-center gap-1">
            <button
              onClick={handleRefresh}
              disabled={refreshing}
              className="rounded-md p-1.5 transition-colors hover:bg-[var(--bg-hover)] disabled:opacity-40"
              style={{ cursor: "pointer", background: "none", border: "none", color: "var(--fill-tertiary)" }}
              title="Refresh skills"
            >
              <ArrowsClockwise size={ICON_SIZE.sm} className={refreshing ? "animate-spin" : ""} />
            </button>
            <div className="relative">
              <button
                onClick={() => setSkillMenuOpen((v) => !v)}
                disabled={uploading}
                className="rounded-md p-1.5 transition-colors hover:bg-[var(--bg-hover)] disabled:opacity-40"
                style={{ cursor: "pointer", background: "none", border: "none", color: "var(--fill-tertiary)" }}
                title="Upload skill"
              >
                <UploadSimple size={ICON_SIZE.sm} />
              </button>
              {skillMenuOpen && (
                <div
                  className="absolute right-0 top-full z-50 mt-1 min-w-[140px] overflow-hidden rounded-md py-1 shadow-lg"
                  style={{ background: "var(--bg-elevated)", border: "0.5px solid var(--separator)" }}
                  onMouseLeave={() => setSkillMenuOpen(false)}
                >
                  <button
                    onClick={() => { setSkillMenuOpen(false); handleUploadFolder(); }}
                    className="flex w-full items-center gap-2 px-3 py-2 text-left text-[12px] transition-colors hover:bg-[var(--bg-hover)]"
                    style={{ cursor: "pointer", background: "none", border: "none", color: "var(--fill-primary)" }}
                  >
                    <FolderOpen size={ICON_SIZE.sm} /> Select Folder
                  </button>
                  <button
                    onClick={() => { setSkillMenuOpen(false); handleUploadZip(); }}
                    className="flex w-full items-center gap-2 px-3 py-2 text-left text-[12px] transition-colors hover:bg-[var(--bg-hover)]"
                    style={{ cursor: "pointer", background: "none", border: "none", color: "var(--fill-primary)" }}
                  >
                    <FileText size={ICON_SIZE.sm} /> Select ZIP
                  </button>
                </div>
              )}
            </div>
          </div>
        )}
      </div>

      {filter === "skills" ? (
        <div className="flex flex-col gap-4">
          {/* Global skills */}
          <div>
            <div className="mb-2 flex items-center gap-2 text-[11px] font-medium" style={{ color: "var(--fill-tertiary)" }}>
              <Globe size={ICON_SIZE.xs} />
              Global Skills ({publicSkills.length})
            </div>
            {publicSkills.length === 0 ? (
              <p className="rounded-md px-4 py-3 text-center text-[12px]" style={{ background: "var(--bg-primary)", border: "0.5px solid var(--separator)", color: "var(--fill-tertiary)" }}>
                No global skills installed
              </p>
            ) : (
              <div className="overflow-hidden rounded-md" style={{ background: "var(--bg-primary)", border: "0.5px solid var(--separator)" }}>
                {publicSkills.map((skill, idx) => (
                  <SkillRow key={skill.id} skill={skill} isLast={idx === publicSkills.length - 1} />
                ))}
              </div>
            )}
          </div>
          {/* Per-agent skills */}
          {Object.entries(agentSkillsMap).map(([agentId, skills]) =>
            skills.length > 0 && (
              <div key={agentId}>
                <div className="mb-2 flex items-center gap-2 text-[11px] font-medium" style={{ color: "var(--fill-tertiary)" }}>
                  <User size={ICON_SIZE.xs} />
                  Agent: {agentId} ({skills.length})
                </div>
                <div className="overflow-hidden rounded-md" style={{ background: "var(--bg-primary)", border: "0.5px solid var(--separator)" }}>
                  {skills.map((skill, idx) => (
                    <SkillRow key={`${agentId}-${skill.id}`} skill={skill} isLast={idx === skills.length - 1} />
                  ))}
                </div>
              </div>
            ),
          )}
        </div>
      ) : (
        tools.length === 0 ? (
          <p className="py-12 text-center text-[13px]" style={{ color: "var(--fill-tertiary)" }}>No registered tools</p>
        ) : (
          <div className="overflow-hidden rounded-md" style={{ background: "var(--bg-primary)", border: "0.5px solid var(--separator)" }}>
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
                <span className="text-[11px] font-mono" style={{ color: "var(--fill-quaternary)" }}>{tool.id}</span>
              </div>
            ))}
          </div>
        )
      )}
    </div>
  );
}

function SkillRow({ skill, isLast }: { skill: api.SkillInfo; isLast: boolean }) {
  return (
    <div
      className="px-4 py-2.5 transition-colors duration-100 hover:bg-[var(--bg-hover)]"
      style={!isLast ? { borderBottom: "0.5px solid var(--separator)" } : undefined}
    >
      <div className="flex items-baseline gap-2">
        <span className="break-all text-[13px] font-semibold leading-snug" style={{ color: "var(--fill-primary)" }}>{skill.name}</span>
        {skill.version && <span className="shrink-0 text-[11px]" style={{ color: "var(--fill-quaternary)" }}>v{skill.version}</span>}
      </div>
      {skill.description && (
        <div className="mt-0.5 line-clamp-2 text-[11px] leading-relaxed" style={{ color: "var(--fill-tertiary)" }}>{skill.description}</div>
      )}
      {skill.tags && skill.tags.length > 0 && (
        <div className="mt-1 flex flex-wrap gap-1">
          {skill.tags.map((tag) => (
            <span key={tag} className="rounded-full px-1.5 py-0.5 text-[11px]" style={{ background: "var(--bg-tertiary)", color: "var(--fill-tertiary)" }}>
              {tag}
            </span>
          ))}
        </div>
      )}
    </div>
  );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Channels Tab
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

const CH_STATUS_CONFIG: Record<string, { label: string; bg: string; fg: string }> = {
  connected: { label: "Connected", bg: "rgba(72,187,120,0.12)", fg: "var(--green)" },
  disconnected: { label: "Disconnected", bg: "var(--bg-tertiary)", fg: "var(--fill-quaternary)" },
  configured: { label: "Configured", bg: "rgba(237,137,54,0.12)", fg: "var(--yellow)" },
  available: { label: "Available", bg: "var(--bg-tertiary)", fg: "var(--fill-quaternary)" },
};

const CH_CAP_LABELS: Record<string, string> = {
  directMessage: "Direct Message",
  groupChat: "Group Chat",
  media: "Media",
  streaming: "Streaming",
  reactions: "Reactions",
  threads: "Threads",
};

const EDITABLE_CONFIG_KEYS = ["appId", "appSecret", "verificationToken", "encryptKey", "domain", "replyMode"];

function ChannelsTabContent({ onCountChange }: { onCountChange: (n: number) => void }) {
  const { t } = useTranslation("plugins");
  const [channels, setChannels] = useState<ChannelStatus[]>([]);
  const [loading, setLoading] = useState(true);
  const [showWechatQr, setShowWechatQr] = useState(false);
  const [channelDetailId, setChannelDetailId] = useState<string | null>(null);

  const fetchChannels = useCallback(async () => {
    try {
      const ch = await api.listChannels();
      setChannels(ch);
      setLoading(false);
    } catch {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    fetchChannels();
    const unsub = api.onChannelsChanged?.(() => fetchChannels());
    return () => unsub?.();
  }, [fetchChannels]);

  useEffect(() => {
    onCountChange(channels.length);
  }, [channels.length, onCountChange]);

  const handleConnect = useCallback(async (ch: ChannelStatus) => {
    if (ch.id === "wechat") {
      setShowWechatQr(true);
      return;
    }
    if (ch.status === "configured") {
      try {
        const result = await api.channelsConnect(ch.id);
        if (result.ok) {
          await fetchChannels();
        } else {
          setChannelDetailId(ch.id);
        }
      } catch {
        setChannelDetailId(ch.id);
      }
    } else {
      setChannelDetailId(ch.id);
    }
  }, [fetchChannels]);

  const handleDisconnect = useCallback(async (ch: ChannelStatus) => {
    try {
      await api.channelsDisconnect(ch.id);
      await fetchChannels();
    } catch (e) {
      console.warn("[channels] disconnect error:", e);
    }
  }, [fetchChannels]);

  const handleWechatSuccess = useCallback(async () => {
    setShowWechatQr(false);
    await fetchChannels();
  }, [fetchChannels]);

  if (loading) {
    return (
      <div className="flex flex-col items-center justify-center gap-3 py-20 pv-fade-in">
        <SpinnerGap size={ICON_SIZE.lg} className="animate-spin" style={{ color: "var(--fill-quaternary)" }} />
        <p className="text-xs" style={{ color: "var(--fill-quaternary)" }}>{t("loading_channels")}</p>
      </div>
    );
  }

  if (channels.length === 0) {
    return (
      <div className="flex flex-col items-center justify-center gap-5 py-24 pv-fade-in">
        <div
          className="pv-float flex h-16 w-16 items-center justify-center rounded-[16px]"
          style={{ background: "color-mix(in srgb, var(--tint) 6%, transparent)" }}
        >
          <WifiSlash size={ICON_SIZE["2xl"]} style={{ color: "var(--tint)", opacity: 0.8 }} />
        </div>
        <div className="text-center">
          <p className="text-[16px] font-semibold" style={{ color: "var(--fill-primary)" }}>{t("no_channels_title")}</p>
          <p className="mt-2 text-[13px] leading-relaxed" style={{ color: "var(--fill-quaternary)", maxWidth: 320 }}>
            {t("no_channels_desc")}
          </p>
        </div>
      </div>
    );
  }

  return (
    <div className="mx-auto w-full max-w-[var(--content-max-w)] px-6 py-5 pv-fade-in">
      <div className="flex flex-col gap-2">
        {channels.map((ch) => (
          <ChannelCard
            key={ch.id}
            channel={ch}
            onConnect={handleConnect}
            onDisconnect={handleDisconnect}
            onClick={setChannelDetailId}
          />
        ))}
      </div>

      <WechatQrModal
        open={showWechatQr}
        onClose={() => setShowWechatQr(false)}
        onSuccess={handleWechatSuccess}
      />

      <ChannelDetailModal
        open={channelDetailId !== null}
        channelId={channelDetailId ?? ""}
        onClose={() => setChannelDetailId(null)}
        onConnect={(id) => {
          const ch = channels.find((c) => c.id === id);
          if (ch) handleConnect(ch);
        }}
        onDisconnect={(id) => {
          const ch = channels.find((c) => c.id === id);
          if (ch) handleDisconnect(ch);
        }}
        onUpdated={fetchChannels}
      />
    </div>
  );
}

function ChannelCard({
  channel,
  onConnect,
  onDisconnect,
  onClick,
}: {
  channel: ChannelStatus;
  onConnect: (ch: ChannelStatus) => void;
  onDisconnect: (ch: ChannelStatus) => void;
  onClick: (id: string) => void;
}) {
  const [disconnecting, setDisconnecting] = useState(false);
  const connected = channel.status === "connected";
  const activeCaps = Object.entries(channel.capabilities ?? {})
    .filter(([, v]) => v)
    .map(([k]) => k);
  const statusCfg = CH_STATUS_CONFIG[channel.status] ?? CH_STATUS_CONFIG.available;

  const handleDisconnect = async () => {
    setDisconnecting(true);
    onDisconnect(channel);
    setDisconnecting(false);
  };

  return (
    <div
      className="flex cursor-pointer items-center gap-3 rounded-[var(--radius-sm)] px-4 py-3.5 transition-colors duration-150 hover:bg-[var(--bg-hover)]"
      style={{ border: "0.5px solid var(--separator)" }}
      onClick={() => onClick(channel.id)}
    >
      <StatusDot status={connected ? "connected" : "disconnected"} />
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2">
          <span className="text-[14px] font-semibold" style={{ color: "var(--fill-primary)" }}>{channel.name}</span>
          <span className="rounded-full px-1.5 py-0.5 text-[11px] font-medium" style={{ background: statusCfg.bg, color: statusCfg.fg }}>
            {statusCfg.label}
          </span>
          {channel.connectionMode && (
            <span className="text-[11px]" style={{ color: "var(--fill-quaternary)" }}>{channel.connectionMode}</span>
          )}
        </div>
        <p className="mt-0.5 truncate text-[11px]" style={{ color: "var(--fill-tertiary)" }}>{channel.description}</p>
        {activeCaps.length > 0 && (
          <div className="mt-1.5 flex flex-wrap gap-1">
            {activeCaps.map((cap) => (
              <span key={cap} className="rounded-full px-1.5 py-0.5 text-[11px]" style={{ background: "var(--bg-tertiary)", color: "var(--fill-tertiary)" }}>
                {CH_CAP_LABELS[cap] ?? cap}
              </span>
            ))}
          </div>
        )}
      </div>
      {connected ? (
        <button
          onClick={(e) => { e.stopPropagation(); handleDisconnect(); }}
          disabled={disconnecting}
          className="flex items-center gap-1 rounded-md px-2 py-1 text-[11px] font-medium transition-colors hover:bg-[var(--bg-hover)]"
          style={{ cursor: "pointer", background: "none", border: "none", color: "var(--red)" }}
        >
          <LinkBreak size={ICON_SIZE.xs} /> Disconnect
        </button>
      ) : (
        <button
          onClick={(e) => { e.stopPropagation(); onConnect(channel); }}
          className="flex items-center gap-1 rounded-md px-2 py-1 text-[11px] font-medium"
          style={{ cursor: "pointer", background: "var(--tint)", border: "none", color: "#fff" }}
        >
          <Link size={ICON_SIZE.xs} /> Connect
        </button>
      )}
    </div>
  );
}

// ─── WechatQrModal ───

type QrStep = "idle" | "loading" | "scanning" | "scanned" | "verify_code" | "confirmed" | "error";

function WechatQrModal({ open, onClose, onSuccess }: { open: boolean; onClose: () => void; onSuccess: () => void }) {
  const [step, setStep] = useState<QrStep>("idle");
  const [qrUrl, setQrUrl] = useState("");
  const [sessionKey, setSessionKey] = useState("");
  const [verifyCode, setVerifyCode] = useState("");
  const [message, setMessage] = useState("");
  const pollRef = useState<ReturnType<typeof setInterval> | null>(null);

  const cleanup = useCallback(() => {
    if (pollRef[0]) { clearInterval(pollRef[0]); pollRef[0] = null; }
  }, [pollRef]);

  useEffect(() => {
    if (!open) {
      cleanup();
      setStep("idle");
      setQrUrl("");
      setSessionKey("");
      setVerifyCode("");
      setMessage("");
    }
  }, [open, cleanup]);

  useEffect(() => () => cleanup(), [cleanup]);

  const startLogin = async () => {
    setStep("loading");
    try {
      const resp = await api.channelsWechatLogin();
      if (!resp.sessionKey) { setStep("error"); setMessage("Cannot get QR code"); return; }
      setSessionKey(resp.sessionKey);
      setQrUrl(resp.qrUrl);
      setStep("scanning");
      pollRef[0] = setInterval(async () => {
        try {
          const poll = await api.channelsWechatPoll(resp.sessionKey);
          switch (poll.status) {
            case "waiting": break;
            case "scanned": setStep("scanned"); break;
            case "need_verify_code": setStep("verify_code"); setMessage(poll.message ?? "Enter pair code"); cleanup(); break;
            case "confirmed":
            case "already_connected":
              setStep("confirmed"); setMessage(poll.message ?? "Connected!"); cleanup(); setTimeout(() => onSuccess(), 1500); break;
            case "expired_refreshed": if (poll.qrUrl) setQrUrl(poll.qrUrl); setStep("scanning"); break;
            default: setStep("error"); setMessage(poll.message ?? "Connection failed"); cleanup();
          }
        } catch { setStep("error"); setMessage("Poll failed"); cleanup(); }
      }, 1500);
    } catch { setStep("error"); setMessage("Start failed"); }
  };

  const submitVerifyCode = async () => {
    if (!verifyCode.trim()) return;
    await api.channelsWechatVerify(sessionKey, verifyCode.trim());
    setStep("scanning");
    setVerifyCode("");
    pollRef[0] = setInterval(async () => {
      try {
        const poll = await api.channelsWechatPoll(sessionKey);
        if (poll.status === "confirmed") {
          setStep("confirmed"); setMessage(poll.message ?? "Connected!"); cleanup(); setTimeout(() => onSuccess(), 1500);
        } else if (poll.status === "verify_blocked") {
          setStep("verify_code"); setMessage("Code rejected"); cleanup();
        } else if (poll.status !== "waiting" && poll.status !== "scanned") {
          setStep("error"); setMessage(poll.message ?? "Connection failed"); cleanup();
        }
      } catch { cleanup(); }
    }, 1500);
  };

  if (!open) return null;

  const inputStyle: React.CSSProperties = {
    background: "var(--bg-primary)",
    border: "0.5px solid var(--separator)",
    color: "var(--fill-primary)",
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center" style={{ background: "rgba(0,0,0,0.5)" }} onClick={onClose}>
      <div className="w-[400px] rounded-[var(--radius-lg)] p-6" style={{ background: "var(--bg-card)", border: "0.5px solid var(--separator)" }} onClick={(e) => e.stopPropagation()}>
        <div className="mb-5 flex items-center justify-between">
          <h3 className="text-[14px] font-semibold" style={{ color: "var(--fill-primary)" }}>WeChat Login</h3>
          <button onClick={onClose} style={{ cursor: "pointer", background: "none", border: "none", color: "var(--fill-tertiary)" }}><X size={ICON_SIZE.md} /></button>
        </div>

        {step === "idle" && (
          <div className="flex flex-col items-center gap-4 py-6">
            <div className="flex h-16 w-16 items-center justify-center rounded-full" style={{ background: "rgba(72,187,120,0.1)" }}>
              <QrCode size={ICON_SIZE.xl} style={{ color: "var(--green)" }} />
            </div>
            <p className="text-center text-[13px]" style={{ color: "var(--fill-secondary)" }}>Scan QR code with WeChat to connect</p>
            <button onClick={startLogin} className="rounded-md px-4 py-2 text-[13px] font-medium" style={{ cursor: "pointer", background: "var(--tint)", border: "none", color: "#fff" }}>Get QR Code</button>
          </div>
        )}

        {step === "loading" && (
          <div className="flex flex-col items-center gap-3 py-8">
            <SpinnerGap size={ICON_SIZE.xl} className="animate-spin" style={{ color: "var(--tint)" }} />
            <p className="text-[12px]" style={{ color: "var(--fill-tertiary)" }}>Fetching QR code…</p>
          </div>
        )}

        {(step === "scanning" || step === "scanned") && (
          <div className="flex flex-col items-center gap-4 py-2">
            {qrUrl ? (
              <div className="rounded-md p-3" style={{ background: "#fff" }}>
                <img src={qrUrl} alt="WeChat QR Code" className="h-48 w-48" />
              </div>
            ) : (
              <div className="flex h-48 w-48 items-center justify-center rounded-md bg-white"><QrCode size={ICON_SIZE["2xl"]} style={{ color: "#ccc" }} /></div>
            )}
            <div className="flex items-center gap-2">
              {step === "scanned" ? (
                <><DeviceMobile size={ICON_SIZE.sm} style={{ color: "var(--green)" }} /><p className="text-[13px] font-medium" style={{ color: "var(--green)" }}>Scanned — confirm on phone</p></>
              ) : (
                <><QrCode size={ICON_SIZE.sm} style={{ color: "var(--fill-tertiary)" }} /><p className="text-[13px]" style={{ color: "var(--fill-secondary)" }}>Scan QR code with WeChat</p></>
              )}
            </div>
          </div>
        )}

        {step === "verify_code" && (
          <div className="flex flex-col items-center gap-4 py-4">
            <div className="flex h-12 w-12 items-center justify-center rounded-full" style={{ background: "rgba(237,137,54,0.1)" }}>
              <Key size={ICON_SIZE.lg} style={{ color: "var(--yellow)" }} />
            </div>
            <p className="text-center text-[13px]" style={{ color: "var(--fill-secondary)" }}>{message}</p>
            <input value={verifyCode} onChange={(e) => setVerifyCode(e.target.value)} placeholder="Enter code"
              className="w-32 rounded-md px-3 py-2 text-center text-[16px] font-mono tracking-wider outline-none" style={inputStyle}
              autoFocus onKeyDown={(e) => e.key === "Enter" && submitVerifyCode()} />
            <button onClick={submitVerifyCode} disabled={!verifyCode.trim()} className="rounded-md px-4 py-1.5 text-[12px] font-medium disabled:opacity-40" style={{ cursor: "pointer", background: "var(--tint)", border: "none", color: "#fff" }}>Submit</button>
          </div>
        )}

        {step === "confirmed" && (
          <div className="flex flex-col items-center gap-3 py-8">
            <CheckCircle size={ICON_SIZE["2xl"]} style={{ color: "var(--green)" }} />
            <p className="text-[14px] font-medium" style={{ color: "var(--green)" }}>{message}</p>
          </div>
        )}

        {step === "error" && (
          <div className="flex flex-col items-center gap-4 py-6">
            <p className="text-[13px]" style={{ color: "var(--red)" }}>{message}</p>
            <button onClick={startLogin} className="rounded-md px-4 py-1.5 text-[12px] font-medium" style={{ cursor: "pointer", background: "var(--tint)", border: "none", color: "#fff" }}>Retry</button>
          </div>
        )}
      </div>
    </div>
  );
}

// ─── ChannelDetailModal ───

function ChannelDetailModal({
  open, channelId, onClose, onConnect, onDisconnect, onUpdated,
}: {
  open: boolean; channelId: string; onClose: () => void;
  onConnect: (id: string) => void; onDisconnect: (id: string) => void; onUpdated: () => void;
}) {
  const [data, setData] = useState<ChannelDetailResult | null>(null);
  const [loading, setLoading] = useState(false);
  const [editing, setEditing] = useState(false);
  const [editValues, setEditValues] = useState<Record<string, string>>({});
  const [saving, setSaving] = useState(false);
  const [restoring, setRestoring] = useState(false);
  const [saveMsg, setSaveMsg] = useState<{ ok: boolean; text: string } | null>(null);
  const [toolSearch, setToolSearch] = useState("");
  const [toolsExpanded, setToolsExpanded] = useState(true);

  useEffect(() => {
    if (!open || !channelId) return;
    setLoading(true); setEditing(false); setSaveMsg(null); setToolSearch(""); setToolsExpanded(true);
    api.channelsDetail(channelId).then((d) => { setData(d); setLoading(false); });
  }, [open, channelId]);

  const startEdit = () => {
    if (!data) return;
    const vals: Record<string, string> = {};
    for (const k of EDITABLE_CONFIG_KEYS) { const v = data.config[k]; vals[k] = v != null ? String(v) : ""; }
    setEditValues(vals);
    setEditing(true);
    setSaveMsg(null);
  };

  const handleSave = async () => {
    setSaving(true); setSaveMsg(null);
    const config: Record<string, unknown> = {};
    for (const [k, v] of Object.entries(editValues)) { if (v.trim()) config[k] = v.trim(); }
    const result = await api.channelsUpdate(channelId, config);
    setSaving(false);
    if (result.ok) {
      setSaveMsg({ ok: true, text: "Saved & reloaded" }); setEditing(false); onUpdated();
      api.channelsDetail(channelId).then(setData);
    } else {
      setSaveMsg({ ok: false, text: result.reloadError ?? "Save failed" });
    }
  };

  const handleRestore = async () => {
    setRestoring(true); setSaveMsg(null);
    const result = await api.channelsRestore(channelId);
    setRestoring(false);
    if (result.ok) {
      setSaveMsg({ ok: true, text: "Restored & reloaded" }); setEditing(false); onUpdated();
      api.channelsDetail(channelId).then(setData);
    } else {
      setSaveMsg({ ok: false, text: result.reloadError ?? "Restore failed" });
    }
  };

  if (!open) return null;

  const connected = data?.status === "connected";
  const statusCfg = data ? (CH_STATUS_CONFIG[data.status] ?? CH_STATUS_CONFIG.available) : null;
  const configEntries = data?.config
    ? Object.entries(data.config).filter(([k, v]) => v != null && v !== "" && typeof v !== "object" && k !== "hasBackup")
    : [];
  const filteredTools = data
    ? data.tools.filter((t) => !toolSearch || t.name.toLowerCase().includes(toolSearch.toLowerCase()) || (t.description && t.description.toLowerCase().includes(toolSearch.toLowerCase())))
    : [];

  const inputStyle: React.CSSProperties = { background: "var(--bg-primary)", border: "0.5px solid var(--separator)", color: "var(--fill-primary)" };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center" style={{ background: "rgba(0,0,0,0.5)" }} onClick={onClose}>
      <div className="flex max-h-[80vh] w-[480px] flex-col rounded-[var(--radius-lg)]" style={{ background: "var(--bg-card)", border: "0.5px solid var(--separator)" }} onClick={(e) => e.stopPropagation()}>
        <div className="flex items-center justify-between px-5 pt-5 pb-3">
          <div className="flex items-center gap-2">
            <WifiHigh size={ICON_SIZE.md} style={{ color: "var(--fill-secondary)" }} />
            <h3 className="text-[14px] font-semibold" style={{ color: "var(--fill-primary)" }}>{data?.name ?? channelId}</h3>
            {statusCfg && <span className="rounded-full px-1.5 py-0.5 text-[11px]" style={{ background: statusCfg.bg, color: statusCfg.fg }}>{statusCfg.label}</span>}
          </div>
          <button onClick={onClose} style={{ cursor: "pointer", background: "none", border: "none", color: "var(--fill-tertiary)" }}><X size={ICON_SIZE.md} /></button>
        </div>

        <div className="flex-1 overflow-y-auto px-5 pb-5">
          {loading ? (
            <div className="flex items-center justify-center py-12"><SpinnerGap size={ICON_SIZE.lg} className="animate-spin" style={{ color: "var(--tint)" }} /></div>
          ) : data ? (
            <div className="flex flex-col gap-4">
              <p className="text-[12px]" style={{ color: "var(--fill-tertiary)" }}>{data.description}</p>

              {/* Config section */}
              <div>
                <div className="mb-2 flex items-center justify-between">
                  <span className="text-[11px] font-medium uppercase tracking-wider" style={{ color: "var(--fill-quaternary)" }}>Configuration</span>
                  {!editing && (
                    <button onClick={startEdit} className="flex items-center gap-1 rounded-md px-1.5 py-0.5 text-[11px] font-medium transition-colors hover:bg-[var(--bg-hover)]" style={{ cursor: "pointer", background: "none", border: "none", color: "var(--fill-tertiary)" }}>
                      <PencilSimple size={ICON_SIZE.xs} /> Edit
                    </button>
                  )}
                </div>

                {editing ? (
                  <div className="flex flex-col gap-2 rounded-md p-3" style={{ background: "var(--bg-primary)", border: "0.5px solid var(--tint)" }}>
                    {EDITABLE_CONFIG_KEYS.map((k) => (
                      <div key={k}>
                        <label className="mb-0.5 block text-[11px] font-medium" style={{ color: "var(--fill-quaternary)" }}>{k}</label>
                        <input value={editValues[k] ?? ""} onChange={(e) => setEditValues((prev) => ({ ...prev, [k]: e.target.value }))}
                          className="w-full rounded-md px-2 py-1.5 text-[12px] font-mono outline-none" style={inputStyle}
                          placeholder={k.includes("Secret") || k.includes("Key") || k.includes("Token") ? "••••••" : ""} />
                      </div>
                    ))}
                    <div className="mt-1 flex items-center gap-2">
                      <button onClick={handleSave} disabled={saving} className="flex items-center gap-1 rounded-md px-3 py-1.5 text-[11px] font-medium disabled:opacity-40" style={{ cursor: "pointer", background: "var(--tint)", border: "none", color: "#fff" }}>
                        <FloppyDisk size={ICON_SIZE.xs} /> {saving ? "Saving…" : "Save & Reload"}
                      </button>
                      {data.hasBackup && (
                        <button onClick={handleRestore} disabled={restoring} className="flex items-center gap-1 rounded-md px-2 py-1.5 text-[11px] font-medium transition-colors hover:bg-[var(--bg-hover)] disabled:opacity-40" style={{ cursor: "pointer", background: "none", border: "none", color: "var(--fill-tertiary)" }}>
                          <ArrowCounterClockwise size={ICON_SIZE.xs} /> Restore
                        </button>
                      )}
                      <button onClick={() => { setEditing(false); setSaveMsg(null); }} className="ml-auto text-[11px] transition-colors hover:bg-[var(--bg-hover)]" style={{ cursor: "pointer", background: "none", border: "none", color: "var(--fill-quaternary)" }}>Cancel</button>
                    </div>
                  </div>
                ) : configEntries.length > 0 ? (
                  <div className="flex flex-col gap-1.5 rounded-md p-3 text-[12px] font-mono" style={{ background: "var(--bg-primary)", border: "0.5px solid var(--separator)" }}>
                    {configEntries.map(([k, v]) => (
                      <div key={k} className="flex gap-2">
                        <span style={{ color: "var(--fill-quaternary)" }}>{k}</span>
                        <span style={{ color: "var(--fill-primary)" }}>{String(v)}</span>
                      </div>
                    ))}
                  </div>
                ) : (
                  <div className="rounded-md py-4 text-center text-[11px]" style={{ background: "var(--bg-primary)", border: "0.5px solid var(--separator)", color: "var(--fill-quaternary)" }}>Not configured</div>
                )}
              </div>

              {saveMsg && (
                <div className="rounded-md px-3 py-2 text-[11px]" style={{ background: saveMsg.ok ? "rgba(72,187,120,0.08)" : "rgba(229,62,62,0.08)", color: saveMsg.ok ? "var(--green)" : "var(--red)" }}>{saveMsg.text}</div>
              )}

              {/* Tools section */}
              {data.tools.length > 0 && (
                <div>
                  <div className="mb-2 flex items-center justify-between">
                    <button className="flex items-center gap-1" onClick={() => setToolsExpanded((v) => !v)} style={{ cursor: "pointer", background: "none", border: "none", color: "var(--fill-quaternary)" }}>
                      <span className="text-[11px] font-medium uppercase tracking-wider">Tools ({data.tools.length})</span>
                      {toolsExpanded ? <CaretUp size={ICON_SIZE.xs} /> : <CaretDown size={ICON_SIZE.xs} />}
                    </button>
                    {toolsExpanded && data.tools.length > 5 && (
                      <div className="flex items-center gap-1 rounded-md px-2 py-1" style={{ background: "var(--bg-primary)", border: "0.5px solid var(--separator)" }}>
                        <MagnifyingGlass size={ICON_SIZE.xs} style={{ color: "var(--fill-quaternary)" }} />
                        <input value={toolSearch} onChange={(e) => setToolSearch(e.target.value)} placeholder="Search…" className="w-24 bg-transparent text-[11px] outline-none" style={{ color: "var(--fill-primary)" }} />
                      </div>
                    )}
                  </div>
                  {toolsExpanded && (
                    <div className="max-h-[200px] overflow-y-auto rounded-md" style={{ background: "var(--bg-primary)", border: "0.5px solid var(--separator)" }}>
                      {filteredTools.length === 0 ? (
                        <div className="py-6 text-center text-[12px]" style={{ color: "var(--fill-quaternary)" }}>{toolSearch ? "No matching tools" : "No tools"}</div>
                      ) : filteredTools.map((t, i) => (
                        <div key={t.name} className="flex items-start gap-2 px-3 py-2" style={{ borderBottom: i < filteredTools.length - 1 ? "0.5px solid var(--separator)" : undefined }}>
                          <Terminal size={ICON_SIZE.xs} className="mt-0.5 shrink-0" style={{ color: "var(--fill-quaternary)" }} />
                          <div>
                            <div className="text-[12px] font-medium" style={{ color: "var(--fill-primary)" }}>{t.name}</div>
                            {t.description && <div className="mt-0.5 text-[11px]" style={{ color: "var(--fill-tertiary)" }}>{t.description}</div>}
                          </div>
                        </div>
                      ))}
                    </div>
                  )}
                </div>
              )}

              {/* Footer actions */}
              <div className="flex items-center gap-2 border-t pt-3" style={{ borderColor: "var(--separator)" }}>
                {connected ? (
                  <button onClick={() => { onDisconnect(channelId); onClose(); }} className="flex items-center gap-1 rounded-md px-3 py-1.5 text-[12px] font-medium transition-colors hover:bg-[var(--bg-hover)]" style={{ cursor: "pointer", background: "none", border: "none", color: "var(--red)" }}>
                    <LinkBreak size={ICON_SIZE.xs} /> Disconnect
                  </button>
                ) : configEntries.length > 0 ? (
                  <button onClick={() => { onConnect(channelId); onClose(); }} className="flex items-center gap-1 rounded-md px-3 py-1.5 text-[12px] font-medium" style={{ cursor: "pointer", background: "var(--tint)", border: "none", color: "#fff" }}>
                    <Link size={ICON_SIZE.xs} /> Connect
                  </button>
                ) : (
                  <button onClick={startEdit} className="flex items-center gap-1 rounded-md px-3 py-1.5 text-[12px] font-medium" style={{ cursor: "pointer", background: "var(--tint)", border: "none", color: "#fff" }}>
                    <PencilSimple size={ICON_SIZE.xs} /> Configure & Connect
                  </button>
                )}
              </div>
            </div>
          ) : (
            <div className="py-8 text-center text-[12px]" style={{ color: "var(--fill-quaternary)" }}>Failed to load details</div>
          )}
        </div>
      </div>
    </div>
  );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Shared Components (MCP tab)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

function PluginRow({
  plugin: p, expanded, tools, registryEntry, onToggle, onRestart, onExpand, onRemove, onDetail, onOauthLogin, index,
}: {
  plugin: PluginSummary; expanded: boolean; tools?: PluginTool[]; registryEntry?: McpRegistryEntry;
  onToggle: () => void; onRestart: () => void; onExpand: () => void; onRemove: () => Promise<boolean>; onDetail: () => void; onOauthLogin?: () => void; index: number;
}) {
  const { t } = useTranslation("plugins");
  const [restarting, setRestarting] = useState(false);
  const [loggingIn, setLoggingIn] = useState(false);
  const [confirmRemove, setConfirmRemove] = useState(false);

  const handleRestart = async (e: React.MouseEvent) => {
    e.stopPropagation();
    setRestarting(true);
    await onRestart();
    setRestarting(false);
  };

  const handleRemoveClick = (e: React.MouseEvent) => {
    e.stopPropagation();
    setConfirmRemove(true);
  };

  const handleConfirmRemove = async (e: React.MouseEvent) => {
    e.stopPropagation();
    const ok = await onRemove();
    if (ok !== false) setConfirmRemove(false);
  };

  const handleCancelRemove = (e: React.MouseEvent) => {
    e.stopPropagation();
    setConfirmRemove(false);
  };

  return (
    <div
      className="pv-stagger rounded-[var(--radius-sm)] transition-all duration-200"
      style={{
        "--stagger-i": index,
        background: expanded ? "var(--bg-primary)" : "transparent",
        border: expanded ? "0.5px solid var(--separator)" : "0.5px solid transparent",
      } as React.CSSProperties}
    >
      <div
        className="group flex items-center gap-3 px-4 py-3.5 transition-colors duration-150 hover:bg-[var(--bg-hover)]"
        style={{ cursor: "pointer", borderRadius: "var(--radius-sm)" }}
        onClick={onExpand}
        role="button"
        tabIndex={0}
        onKeyDown={(e) => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); onExpand(); } }}
      >
        {expanded ? <CaretDown size={ICON_SIZE.sm} style={{ color: "var(--fill-quaternary)" }} /> : <CaretRight size={ICON_SIZE.sm} style={{ color: "var(--fill-quaternary)" }} />}
        <PluginIcon entry={registryEntry} status={p.status} />
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-2.5">
            <span
              className="truncate text-[14px] font-semibold transition-colors hover:underline"
              style={{ color: "var(--fill-primary)", cursor: "pointer" }}
              onClick={(e) => { e.stopPropagation(); onDetail(); }}
              role="link"
              tabIndex={0}
              onKeyDown={(e) => { if (e.key === "Enter") { e.stopPropagation(); onDetail(); } }}
            >
              {p.name}
            </span>
            <ScopeBadge scope={p.scope} />
            {p.transport && p.transport !== "stdio" && (
              <span
                className="rounded px-1.5 py-0.5 text-[10px] font-medium uppercase"
                style={{ background: "var(--bg-tertiary)", color: "var(--fill-quaternary)" }}
              >
                {p.transport === "streamable_http" ? "HTTP" : p.transport.toUpperCase()}
              </span>
            )}
          </div>
          {p.status === "needs_auth" && (
            <div className="mt-0.5 flex items-center gap-1 text-[11px]" style={{ color: "var(--yellow, #D69E2E)" }}>
              <Key size={ICON_SIZE.xs} />
              <span>{t("needs_auth_hint", "OAuth authentication required")}</span>
            </div>
          )}
          {p.lastError && p.status !== "needs_auth" && (
            <div className="mt-0.5 flex items-center gap-1 text-[11px]" style={{ color: "var(--red, #E53E3E)" }}>
              <WarningCircle size={ICON_SIZE.xs} />
              <span className="truncate">{p.lastError}</span>
            </div>
          )}
        </div>
        <div className="flex items-center gap-2">
          {p.status === "needs_auth" && onOauthLogin && (
            <button
              onClick={(e) => {
                e.stopPropagation();
                setLoggingIn(true);
                onOauthLogin();
                setTimeout(() => setLoggingIn(false), 3000);
              }}
              disabled={loggingIn}
              className="flex items-center gap-1 rounded-md px-2.5 py-1.5 text-[11px] font-semibold transition-all hover:brightness-110 disabled:opacity-50"
              style={{
                cursor: loggingIn ? "wait" : "pointer",
                background: "var(--yellow, #D69E2E)",
                border: "none",
                color: "#fff",
              }}
            >
              {loggingIn ? (
                <SpinnerGap size={ICON_SIZE.xs} className="animate-spin" />
              ) : (
                <Key size={ICON_SIZE.xs} weight="fill" />
              )}
              {t("oauth_login", "Login")}
            </button>
          )}
          {p.toolCount > 0 && (
            <span className="text-[11px] tabular-nums" style={{ color: "var(--fill-quaternary)" }}>{t("tools_count", { count: p.toolCount })}</span>
          )}
          {confirmRemove ? (
            <div className="flex items-center gap-1" onClick={(e) => e.stopPropagation()}>
              <span className="text-[11px]" style={{ color: "var(--red)" }}>{t("remove_confirm")}</span>
              <button
                onClick={handleConfirmRemove}
                className="rounded-[var(--radius-xs)] px-2 py-1 text-[11px] font-medium"
                style={{ cursor: "pointer", background: "var(--red)", color: "#fff", border: "none" }}
              >
                {t("remove_yes")}
              </button>
              <button
                onClick={handleCancelRemove}
                className="rounded-[var(--radius-xs)] px-2 py-1 text-[11px] font-medium"
                style={{ cursor: "pointer", background: "var(--bg-tertiary)", color: "var(--fill-secondary)", border: "none" }}
              >
                {t("remove_no")}
              </button>
            </div>
          ) : (
            <>
              <button
                onClick={handleRemoveClick}
                className="rounded-[var(--radius-xs)] p-1.5 opacity-0 transition-all duration-200 group-hover:opacity-100 hover:bg-[var(--bg-hover)]"
                style={{ cursor: "pointer", background: "none", border: "none", color: "var(--fill-quaternary)" }}
                title={t("remove_server")}
                aria-label={`Remove ${p.name}`}
              >
                <Trash size={ICON_SIZE.sm} />
              </button>
              <button
                onClick={handleRestart}
                disabled={restarting || !p.enabled}
                className="rounded-[var(--radius-xs)] p-1.5 opacity-0 transition-all duration-200 group-hover:opacity-100 hover:bg-[var(--bg-hover)] disabled:opacity-30"
                style={{ cursor: "pointer", background: "none", border: "none", color: "var(--fill-tertiary)" }}
                title="Restart"
                aria-label={`Restart ${p.name}`}
              >
                <ArrowsClockwise size={ICON_SIZE.sm} className={restarting ? "animate-spin" : ""} />
              </button>
            </>
          )}
          <button
            onClick={(e) => { e.stopPropagation(); onToggle(); }}
            className="rounded-[var(--radius-xs)] p-1.5 transition-all duration-200"
            style={{ cursor: "pointer", background: "none", border: "none" }}
            title={p.enabled ? "Disable" : "Enable"}
            aria-label={`${p.enabled ? "Disable" : "Enable"} ${p.name}`}
          >
            {p.enabled ? <ToggleRight size={ICON_SIZE.lg} style={{ color: "var(--green, #38A169)" }} /> : <ToggleLeft size={ICON_SIZE.lg} style={{ color: "var(--fill-quaternary)" }} />}
          </button>
        </div>
      </div>

      {expanded && (
        <div className="px-4 pb-4 pv-fade-in">
          <div className="ml-6 border-l pl-4 pt-1" style={{ borderColor: "var(--separator)" }}>
            {p.connectedAt && <DetailRow label="Connected" value={new Date(p.connectedAt).toLocaleString()} />}
            <DetailRow label="Status" value={p.status} />
            {p.lastError && <DetailRow label="Error" value={p.lastError} isError />}
            {tools && tools.length > 0 && (
              <div className="mt-3">
                <p className="mb-2 flex items-center gap-1.5 text-[11px] font-semibold uppercase tracking-wider" style={{ color: "var(--fill-quaternary)" }}>
                  <Wrench size={ICON_SIZE.xs} /> Tools ({tools.length})
                </p>
                <div className="flex flex-col gap-1.5">
                  {tools.map((t) => (
                    <div key={t.name} className="rounded-[var(--radius-xs)] px-3 py-2" style={{ background: "var(--bg-card)" }}>
                      <p className="text-[12px] font-medium" style={{ color: "var(--fill-primary)" }}>{t.name}</p>
                      {t.description && <p className="mt-0.5 text-[11px] leading-relaxed" style={{ color: "var(--fill-quaternary)" }}>{t.description}</p>}
                    </div>
                  ))}
                </div>
              </div>
            )}
            {tools && tools.length === 0 && (
              <p className="mt-1 text-[11px]" style={{ color: "var(--fill-quaternary)" }}>No tools available</p>
            )}
          </div>
        </div>
      )}
    </div>
  );
}

function PluginIcon({ entry, status }: { entry?: McpRegistryEntry; status: string }) {
  const IconComp = entry ? (PLUGIN_ICON_MAP[entry.icon] ?? PuzzlePiece) : PuzzlePiece;
  const brand = entry?.brandColor ?? "var(--tint)";
  const statusColor =
    status === "connected" ? "var(--green)" :
    status === "failed" ? "var(--red)" :
    status === "connecting" ? "var(--orange)" :
    status === "pending_approval" ? "var(--yellow)" :
    status === "needs_auth" ? "var(--yellow)" : "var(--fill-quaternary)";
  return (
    <div className="relative shrink-0">
      <div
        className="flex h-7 w-7 items-center justify-center rounded-[7px]"
        style={{ background: `color-mix(in srgb, ${brand} 10%, transparent)` }}
      >
        <IconComp size={ICON_SIZE.sm} style={{ color: brand }} />
      </div>
      <span
        className="absolute -bottom-0.5 -right-0.5 h-2 w-2 rounded-full"
        style={{ background: statusColor, border: "1.5px solid var(--bg-card)" }}
      />
    </div>
  );
}

function StatusDot({ status }: { status: string }) {
  const isConnecting = status === "connecting";
  const color =
    status === "connected" ? "var(--green, #38A169)" :
    status === "failed" ? "var(--red, #E53E3E)" :
    isConnecting ? "var(--orange, #ED8936)" :
    status === "pending_approval" ? "var(--yellow, #D69E2E)" :
    status === "needs_auth" ? "var(--yellow, #D69E2E)" :
    "var(--fill-quaternary)";
  const shouldPulse = isConnecting || status === "pending_approval" || status === "needs_auth";
  return (
    <span className="relative flex h-2.5 w-2.5 shrink-0">
      {shouldPulse && <span className="absolute inline-flex h-full w-full animate-ping rounded-full opacity-40" style={{ background: color }} />}
      <span className="relative inline-flex h-2.5 w-2.5 rounded-full" style={{ background: color }} />
    </span>
  );
}

function ScopeBadge({ scope }: { scope: string }) {
  return (
    <span
      className="shrink-0 rounded px-1.5 py-0.5 text-[11px] font-semibold uppercase tracking-wide"
      style={{
        background: scope === "project" ? "color-mix(in srgb, var(--orange, #ED8936) 8%, transparent)" : "var(--bg-tertiary)",
        color: scope === "project" ? "var(--orange, #ED8936)" : "var(--fill-quaternary)",
      }}
    >
      {scope}
    </span>
  );
}

function DetailRow({ label, value, isError }: { label: string; value: string; isError?: boolean }) {
  return (
    <div className="flex items-start gap-3 py-0.5">
      <span className="shrink-0 text-[11px] font-medium" style={{ color: "var(--fill-quaternary)", minWidth: 72 }}>{label}</span>
      <span className="text-[12px] break-all" style={{ color: isError ? "var(--red, #E53E3E)" : "var(--fill-secondary)" }}>{value}</span>
    </div>
  );
}

