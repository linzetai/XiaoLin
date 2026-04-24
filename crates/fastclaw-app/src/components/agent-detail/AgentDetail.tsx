import { useState, useEffect, useCallback, useMemo } from "react";
import { useAgentStore } from "../../lib/agent-store";
import { useGatewayStore } from "../../lib/store";
import {
  ChevronDown, ChevronRight, X, FolderOpen, Search, Trash2,
  FileText, User, Shield, AlertTriangle, Link2, Plus, Pencil, Camera,
  RefreshCw, Upload, Clock,
} from "lucide-react";
import * as api from "../../lib/api";
import type { CronJob, CronJobAction, CronJobRun } from "../../lib/transport";
import * as transport from "../../lib/transport";

interface AgentDetailProps {
  open: boolean;
  onClose: () => void;
  agentName: string;
  agentInitial: string;
  agentColor: string;
}

type Tab = "config" | "chats" | "cron";

const COLLAPSE_THRESHOLD = 10;

/* ━━━ Shared Components ━━━ */

function SectionHeader({ children, count, total, searchable, query, onQueryChange }: {
  children: React.ReactNode;
  count?: number;
  total?: number;
  searchable?: boolean;
  query?: string;
  onQueryChange?: (v: string) => void;
}) {
  const [showSearch, setShowSearch] = useState(false);
  return (
    <div className="mb-1.5 flex items-center gap-2">
      <label className="text-[11px] font-medium uppercase tracking-wider" style={{ color: "var(--fill-tertiary)" }}>
        {children}
      </label>
      {total != null && (
        <span className="text-[10px]" style={{ color: "var(--fill-quaternary)" }}>
          ({count ?? total}/{total})
        </span>
      )}
      <div className="flex-1" />
      {searchable && (
        showSearch ? (
          <div className="flex items-center gap-1">
            <input
              type="text"
              value={query ?? ""}
              onChange={(e) => onQueryChange?.(e.target.value)}
              placeholder="搜索..."
              className="w-28 bg-transparent text-[11px] outline-none"
              style={{ color: "var(--fill-primary)", borderBottom: "0.5px solid var(--separator)" }}
              autoFocus
            />
            <button onClick={() => { setShowSearch(false); onQueryChange?.(""); }} className="cursor-pointer" style={{ color: "var(--fill-quaternary)" }}>
              <X size={10} strokeWidth={2} />
            </button>
          </div>
        ) : (
          <button onClick={() => setShowSearch(true)} className="cursor-pointer transition-colors duration-100 hover:opacity-70" style={{ color: "var(--fill-quaternary)" }}>
            <Search size={11} strokeWidth={1.5} />
          </button>
        )
      )}
    </div>
  );
}

function Toggle({ checked, onChange, disabled }: { checked: boolean; onChange: (v: boolean) => void; disabled?: boolean }) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      disabled={disabled}
      onClick={() => onChange(!checked)}
      className="relative inline-flex h-5 w-9 shrink-0 cursor-pointer items-center rounded-full transition-colors duration-200 disabled:cursor-not-allowed disabled:opacity-50"
      style={{ background: checked ? "var(--fill-tertiary)" : "var(--bg-tertiary)" }}
    >
      <span
        className="inline-block h-3.5 w-3.5 rounded-full shadow-sm transition-transform duration-200"
        style={{ background: "white", transform: checked ? "translateX(17px)" : "translateX(3px)" }}
      />
    </button>
  );
}

function ListContainer({ children }: { children: React.ReactNode }) {
  return (
    <div className="overflow-hidden rounded-[var(--radius-sm)]" style={{ background: "var(--bg-elevated)", border: "0.5px solid var(--separator-opaque)" }}>
      {children}
    </div>
  );
}

function EmptyRow({ text }: { text: string }) {
  return (
    <div className="px-3 py-3 text-center text-[12px]" style={{ color: "var(--fill-tertiary)" }}>
      {text}
    </div>
  );
}

function FormModal({ open, onClose, title, children }: {
  open: boolean;
  onClose: () => void;
  title: string;
  children: React.ReactNode;
}) {
  if (!open) return null;
  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center"
      style={{ animation: "fade-in 0.15s ease-out" }}
      onKeyDown={(e) => { if (e.key === "Escape") onClose(); }}
      role="dialog"
      aria-modal="true"
      aria-label={title}
    >
      <div className="absolute inset-0" style={{ background: "rgba(0, 0, 0, 0.3)" }} onClick={onClose} role="presentation" />
      <div
        className="relative w-full max-w-[420px] overflow-hidden rounded-[var(--radius-md)]"
        style={{
          background: "var(--bg-elevated)",
          boxShadow: "var(--shadow-lg)",
          animation: "scale-in 0.2s ease-out",
          border: "0.5px solid var(--separator)",
          maxHeight: "calc(100vh - 80px)",
        }}
      >
        <div className="flex items-center justify-between px-5 py-3.5" style={{ borderBottom: "0.5px solid var(--separator)" }}>
          <h3 className="text-[14px] font-semibold" style={{ color: "var(--fill-primary)" }}>{title}</h3>
          <button onClick={onClose} className="flex h-6 w-6 cursor-pointer items-center justify-center rounded-full transition-colors duration-100 hover:bg-[var(--bg-hover)]" style={{ color: "var(--fill-tertiary)" }}>
            <X size={12} strokeWidth={2} />
          </button>
        </div>
        <div className="overflow-y-auto px-5 py-4" style={{ maxHeight: "calc(100vh - 160px)" }}>
          {children}
        </div>
      </div>
    </div>
  );
}

/* ━━━ Identity Files Section ━━━ */

const IDENTITY_FILES = [
  { key: "soul" as const, name: "SOUL.md", desc: "人格与语气", icon: <User size={13} strokeWidth={1.5} /> },
  { key: "user" as const, name: "USER.md", desc: "用户画像", icon: <FileText size={13} strokeWidth={1.5} /> },
  { key: "agents" as const, name: "AGENTS.md", desc: "规则与约束", icon: <Shield size={13} strokeWidth={1.5} /> },
] as const;

function IdentitySection({ agentId, ready }: { agentId: string; ready: boolean }) {
  const [files, setFiles] = useState<api.IdentityFiles>({ soul: null, user: null, agents: null });
  const [expanded, setExpanded] = useState<string | null>(null);

  useEffect(() => {
    if (!ready) return;
    api.getIdentityFiles(agentId).then(setFiles).catch(() => {});
  }, [agentId, ready]);

  return (
    <div>
      <SectionHeader>身份文件</SectionHeader>
      <ListContainer>
        {IDENTITY_FILES.map((f, i) => {
          const content = files[f.key];
          const isExpanded = expanded === f.key;
          const hasContent = content != null && content.trim().length > 0;
          return (
            <div key={f.key} style={i < IDENTITY_FILES.length - 1 ? { borderBottom: "0.5px solid var(--separator)" } : undefined}>
              <button
                className="flex w-full cursor-pointer items-center gap-2.5 px-3 py-2.5 text-left transition-colors duration-100 hover:bg-[var(--bg-hover)]"
                onClick={() => setExpanded(isExpanded ? null : f.key)}
              >
                <span style={{ color: "var(--fill-tertiary)" }}>{f.icon}</span>
                <div className="min-w-0 flex-1">
                  <span className="text-[13px] font-medium" style={{ color: "var(--fill-primary)" }}>{f.name}</span>
                  <span className="ml-2 text-[11px]" style={{ color: "var(--fill-quaternary)" }}>{f.desc}</span>
                </div>
                {hasContent ? (
                  <ChevronRight
                    size={10} strokeWidth={2}
                    className="shrink-0 transition-transform duration-150"
                    style={{ color: "var(--fill-quaternary)", transform: isExpanded ? "rotate(90deg)" : "rotate(0)" }}
                  />
                ) : (
                  <span className="text-[10px]" style={{ color: "var(--fill-quaternary)" }}>(空)</span>
                )}
              </button>
              {isExpanded && hasContent && (
                <div className="border-t px-3 py-2" style={{ borderColor: "var(--separator)", background: "var(--bg-secondary)" }}>
                  <pre className="max-h-48 overflow-auto whitespace-pre-wrap text-[11px] leading-relaxed" style={{ color: "var(--fill-secondary)" }}>
                    {content}
                  </pre>
                </div>
              )}
            </div>
          );
        })}
      </ListContainer>
    </div>
  );
}

/* ━━━ Channel Manager (Per-Agent CRUD) ━━━ */

const CHANNEL_TYPES = [
  { id: "feishu", label: "飞书" },
  { id: "lark", label: "飞书 (Lark)" },
  { id: "slack", label: "Slack" },
  { id: "telegram", label: "Telegram" },
  { id: "discord", label: "Discord" },
  { id: "matrix", label: "Matrix" },
  { id: "msteams", label: "Teams" },
  { id: "whatsapp", label: "WhatsApp" },
] as const;

const CHANNEL_LABEL_MAP: Record<string, string> = Object.fromEntries(CHANNEL_TYPES.map((t) => [t.id, t.label]));

const EMPTY_CHANNEL: api.AgentChannelConfig = {
  enabled: true,
  connectionMode: "websocket",
  domain: "https://open.feishu.cn",
  replyMode: "mention_only",
};

function ChannelForm({
  channelId,
  config,
  isNew,
  existingIds,
  onSave,
  onCancel,
  onDelete,
  saving,
}: {
  channelId: string;
  config: api.AgentChannelConfig;
  isNew: boolean;
  existingIds: string[];
  onSave: (id: string, cfg: api.AgentChannelConfig) => void;
  onCancel: () => void;
  onDelete?: () => void;
  saving: boolean;
}) {
  const [id, setId] = useState(channelId);
  const [form, setForm] = useState(config);
  const patch = (k: keyof api.AgentChannelConfig, v: string | boolean | undefined) =>
    setForm((f) => ({ ...f, [k]: v }));

  const inputCls = "w-full rounded-[6px] px-3 py-2 text-[13px] outline-none transition-colors focus:ring-1 focus:ring-[var(--fill-quaternary)]";
  const inputStyle = { background: "var(--bg-base)", color: "var(--fill-primary)", border: "0.5px solid var(--separator-opaque)" };
  const labelCls = "mb-1 block text-[11px] font-medium";
  const labelStyle = { color: "var(--fill-tertiary)" };

  const duplicate = isNew && existingIds.includes(id);

  return (
    <div className="space-y-3">
      <div className="grid grid-cols-2 gap-3">
        <div>
          <label className={labelCls} style={labelStyle}>渠道类型</label>
          {isNew ? (
            <div className="relative">
              <select
                value={id}
                onChange={(e) => setId(e.target.value)}
                className={inputCls + " cursor-pointer pr-8"}
                style={{ ...inputStyle, WebkitAppearance: "none", appearance: "none" } as React.CSSProperties}
              >
                {CHANNEL_TYPES.map((t) => (
                  <option key={t.id} value={t.id}>{t.label}</option>
                ))}
              </select>
              <ChevronDown size={10} strokeWidth={2} className="pointer-events-none absolute top-1/2 right-3 -translate-y-1/2" style={{ color: "var(--fill-tertiary)" }} />
            </div>
          ) : (
            <div className={inputCls} style={{ ...inputStyle, opacity: 0.7 }}>
              {CHANNEL_LABEL_MAP[id] ?? id}
            </div>
          )}
          {duplicate && <span className="mt-0.5 text-[10px]" style={{ color: "var(--red, #e53e3e)" }}>该类型已存在</span>}
        </div>
        <div>
          <label className={labelCls} style={labelStyle}>启用</label>
          <div className="flex h-[34px] items-center">
            <Toggle checked={form.enabled !== false} onChange={(v) => patch("enabled", v)} />
          </div>
        </div>
      </div>

      <div className="grid grid-cols-2 gap-3">
        <div>
          <label className={labelCls} style={labelStyle}>连接方式</label>
          <div className="relative">
            <select
              value={form.connectionMode ?? "websocket"}
              onChange={(e) => patch("connectionMode", e.target.value)}
              className={inputCls + " cursor-pointer pr-8"}
              style={{ ...inputStyle, WebkitAppearance: "none", appearance: "none" } as React.CSSProperties}
            >
              <option value="websocket">WebSocket</option>
              <option value="webhook">Webhook</option>
            </select>
            <ChevronDown size={10} strokeWidth={2} className="pointer-events-none absolute top-1/2 right-3 -translate-y-1/2" style={{ color: "var(--fill-tertiary)" }} />
          </div>
        </div>
        <div>
          <label className={labelCls} style={labelStyle}>回复模式</label>
          <div className="relative">
            <select
              value={form.replyMode ?? "mention_only"}
              onChange={(e) => patch("replyMode", e.target.value)}
              className={inputCls + " cursor-pointer pr-8"}
              style={{ ...inputStyle, WebkitAppearance: "none", appearance: "none" } as React.CSSProperties}
            >
              <option value="all">全部消息</option>
              <option value="mention_only">仅 @提及</option>
            </select>
            <ChevronDown size={10} strokeWidth={2} className="pointer-events-none absolute top-1/2 right-3 -translate-y-1/2" style={{ color: "var(--fill-tertiary)" }} />
          </div>
        </div>
      </div>

      <div>
        <label className={labelCls} style={labelStyle}>域名</label>
        <input
          value={form.domain ?? ""}
          onChange={(e) => patch("domain", e.target.value)}
          placeholder="https://open.feishu.cn"
          className={inputCls}
          style={inputStyle}
        />
      </div>

      <div className="grid grid-cols-2 gap-3">
        <div>
          <label className={labelCls} style={labelStyle}>App ID</label>
          <input
            value={form.appId ?? ""}
            onChange={(e) => patch("appId", e.target.value)}
            placeholder="cli_xxxxx"
            className={inputCls + " font-mono"}
            style={inputStyle}
          />
        </div>
        <div>
          <label className={labelCls} style={labelStyle}>App Secret</label>
          <input
            type="password"
            value={form.appSecret ?? ""}
            onChange={(e) => patch("appSecret", e.target.value)}
            placeholder="••••••••"
            className={inputCls + " font-mono"}
            style={inputStyle}
          />
        </div>
      </div>

      <div className="grid grid-cols-2 gap-3">
        <div>
          <label className={labelCls} style={labelStyle}>Verification Token</label>
          <input
            type="password"
            value={form.verificationToken ?? ""}
            onChange={(e) => patch("verificationToken", e.target.value)}
            placeholder="可选"
            className={inputCls + " font-mono"}
            style={inputStyle}
          />
        </div>
        <div>
          <label className={labelCls} style={labelStyle}>Encrypt Key</label>
          <input
            type="password"
            value={form.encryptKey ?? ""}
            onChange={(e) => patch("encryptKey", e.target.value)}
            placeholder="可选"
            className={inputCls + " font-mono"}
            style={inputStyle}
          />
        </div>
      </div>

      <div className="flex items-center justify-between pt-1">
        <div>
          {!isNew && onDelete && (
            <button
              onClick={onDelete}
              disabled={saving}
              className="rounded-[6px] px-3 py-1.5 text-[12px] font-medium transition-colors hover:opacity-80"
              style={{ color: "var(--red, #e53e3e)" }}
            >
              删除渠道
            </button>
          )}
        </div>
        <div className="flex items-center gap-2">
          <button
            onClick={onCancel}
            disabled={saving}
            className="cursor-pointer rounded-[6px] px-3 py-1.5 text-[12px] font-medium transition-colors"
            style={{ color: "var(--fill-secondary)" }}
          >
            取消
          </button>
          <button
            onClick={() => onSave(id, form)}
            disabled={saving || !id || duplicate}
            className="cursor-pointer rounded-[6px] px-4 py-1.5 text-[12px] font-medium transition-colors hover:opacity-90 disabled:opacity-50"
            style={{ background: "var(--fill-primary)", color: "var(--fill-inverse)" }}
          >
            {saving ? "保存中..." : "保存"}
          </button>
        </div>
      </div>
    </div>
  );
}

function ChannelManager({ agentId, backendAgent, ready }: {
  agentId: string;
  backendAgent: api.BackendAgent | null;
  ready: boolean;
}) {
  const [channels, setChannels] = useState<Record<string, api.AgentChannelConfig>>({});
  const [editing, setEditing] = useState<string | null>(null);
  const [adding, setAdding] = useState(false);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    setChannels(backendAgent?.channels ?? {});
    setEditing(null);
    setAdding(false);
  }, [backendAgent]);

  const saveChannels = useCallback(async (newChannels: Record<string, api.AgentChannelConfig>) => {
    if (!backendAgent) return false;
    setSaving(true);
    const ok = await api.updateAgent(agentId, { ...backendAgent, channels: newChannels });
    setSaving(false);
    if (ok) setChannels(newChannels);
    return ok;
  }, [agentId, backendAgent]);

  const handleAdd = useCallback(async (id: string, cfg: api.AgentChannelConfig) => {
    const ok = await saveChannels({ ...channels, [id]: cfg });
    if (ok) setAdding(false);
  }, [channels, saveChannels]);

  const handleEdit = useCallback(async (id: string, cfg: api.AgentChannelConfig) => {
    const ok = await saveChannels({ ...channels, [id]: cfg });
    if (ok) setEditing(null);
  }, [channels, saveChannels]);

  const handleDelete = useCallback(async (id: string) => {
    const { [id]: _, ...rest } = channels;
    const ok = await saveChannels(rest);
    if (ok) setEditing(null);
  }, [channels, saveChannels]);

  if (!ready) return null;

  const entries = Object.entries(channels);
  const existingIds = Object.keys(channels);

  return (
    <div>
      <div className="mb-1.5 flex items-center gap-2">
        <label className="text-[11px] font-medium uppercase tracking-wider" style={{ color: "var(--fill-tertiary)" }}>
          渠道
        </label>
        {entries.length > 0 && (
          <span className="text-[10px]" style={{ color: "var(--fill-quaternary)" }}>({entries.length})</span>
        )}
        <div className="flex-1" />
        {!adding && (
          <button
            onClick={() => { setAdding(true); setEditing(null); }}
            className="flex cursor-pointer items-center gap-1 text-[11px] font-medium transition-colors hover:opacity-70"
            style={{ color: "var(--fill-tertiary)" }}
          >
            <Plus size={11} strokeWidth={2} />
            添加
          </button>
        )}
      </div>

      <FormModal open={adding} onClose={() => setAdding(false)} title="添加渠道">
        <ChannelForm
          channelId="feishu"
          config={EMPTY_CHANNEL}
          isNew
          existingIds={existingIds}
          onSave={handleAdd}
          onCancel={() => setAdding(false)}
          saving={saving}
        />
      </FormModal>

      {editing && channels[editing] && (
        <FormModal open onClose={() => setEditing(null)} title={`编辑渠道 — ${CHANNEL_LABEL_MAP[editing] ?? editing}`}>
          <ChannelForm
            channelId={editing}
            config={channels[editing]}
            isNew={false}
            existingIds={existingIds}
            onSave={handleEdit}
            onCancel={() => setEditing(null)}
            onDelete={() => handleDelete(editing)}
            saving={saving}
          />
        </FormModal>
      )}

      {entries.length === 0 ? (
        <ListContainer>
          <div className="px-3 py-4 text-center">
            <Link2 size={16} strokeWidth={1.5} className="mx-auto mb-1.5" style={{ color: "var(--fill-quaternary)" }} />
            <p className="text-[12px]" style={{ color: "var(--fill-tertiary)" }}>
              添加渠道以连接飞书、Slack 等外部平台
            </p>
          </div>
        </ListContainer>
      ) : (
        <ListContainer>
          {entries.map(([chId, cfg], i) => (
              <div
                key={chId}
                className="group cursor-pointer px-3 py-2.5 transition-colors duration-100 hover:bg-[var(--bg-hover)]"
                style={i < entries.length - 1 ? { borderBottom: "0.5px solid var(--separator)" } : undefined}
                onClick={() => { setEditing(chId); setAdding(false); }}
              >
                <div className="flex items-center justify-between gap-2">
                  <div className="flex min-w-0 items-center gap-2">
                    <Link2 size={13} strokeWidth={1.5} style={{ color: "var(--fill-tertiary)" }} />
                    <span className="text-[13px] font-medium" style={{ color: "var(--fill-primary)" }}>
                      {CHANNEL_LABEL_MAP[chId] ?? chId}
                    </span>
                    <span
                      className="inline-block h-[6px] w-[6px] shrink-0 rounded-full"
                      style={{ background: cfg.enabled !== false ? "var(--green, #48bb78)" : "var(--fill-quaternary)" }}
                    />
                  </div>
                  <Pencil size={12} strokeWidth={1.5} className="shrink-0 opacity-0 transition-opacity group-hover:opacity-100" style={{ color: "var(--fill-quaternary)" }} />
                </div>
                {cfg.domain && (
                  <div className="mt-0.5 truncate text-[11px]" style={{ color: "var(--fill-quaternary)" }}>
                    {cfg.domain}
                  </div>
                )}
              </div>
            )
          )}
        </ListContainer>
      )}

      {entries.length > 0 && (
        <p className="mt-1.5 text-[10px]" style={{ color: "var(--fill-quaternary)" }}>
          渠道凭据变更需重启应用后生效
        </p>
      )}
    </div>
  );
}

/* ━━━ Collapsible List ━━━ */

function CollapsibleList<T>({ items, renderItem, emptyText }: {
  items: T[];
  renderItem: (item: T, index: number, isLast: boolean) => React.ReactNode;
  emptyText: string;
}) {
  const [showAll, setShowAll] = useState(false);
  const needsCollapse = items.length > COLLAPSE_THRESHOLD;
  const visible = needsCollapse && !showAll ? items.slice(0, COLLAPSE_THRESHOLD) : items;

  if (items.length === 0) return <ListContainer><EmptyRow text={emptyText} /></ListContainer>;

  return (
    <ListContainer>
      {visible.map((item, i) => renderItem(item, i, !needsCollapse ? i === items.length - 1 : showAll ? i === items.length - 1 : i === visible.length - 1 && !needsCollapse))}
      {needsCollapse && (
        <button
          onClick={() => setShowAll(!showAll)}
          className="w-full cursor-pointer px-3 py-2 text-center text-[11px] font-medium transition-colors duration-100 hover:bg-[var(--bg-hover)]"
          style={{ color: "var(--fill-tertiary)", borderTop: "0.5px solid var(--separator)" }}
        >
          {showAll ? "收起" : `显示全部 (${items.length})`}
        </button>
      )}
    </ListContainer>
  );
}

/* ━━━ Config Tab ━━━ */

function ConfigTab() {
  const encodeModelOption = (provider: string, model: string) => `${provider}::${model}`;
  const decodeModelOption = (value: string) => {
    const sep = value.indexOf("::");
    if (sep < 0) return { provider: null as string | null, model: value };
    return { provider: value.slice(0, sep), model: value.slice(sep + 2) };
  };

  const activeAgentId = useAgentStore((s) => s.activeAgentId);
  const agents = useAgentStore((s) => s.agents);
  const agent = agents.find((a) => a.id === activeAgentId) ?? agents[0];
  const removeAgent = useAgentStore((s) => s.removeAgent);
  const gatewayReady = useGatewayStore((s) => s.connected);

  const [name, setName] = useState(agent?.name ?? "");
  const [selectedModel, setSelectedModel] = useState(agent?.model ?? "");
  const [selectedProvider, setSelectedProvider] = useState<string | null>(null);
  const [fileAccessMode, setFileAccessMode] = useState<api.FileAccessMode>("workspace");
  const [saving, setSaving] = useState(false);
  const [saveMsg, setSaveMsg] = useState("");

  const [models, setModels] = useState<api.ModelInfo[]>([]);
  const [agentTools, setAgentTools] = useState<api.AgentToolInfo[]>([]);
  const [agentSkills, setAgentSkills] = useState<api.SkillInfo[]>([]);
  const [skillsDeny, setSkillsDeny] = useState<string[]>([]);
  const [togglingTool, setTogglingTool] = useState<string | null>(null);
  const [togglingSkill, setTogglingSkill] = useState<string | null>(null);

  const [toolQuery, setToolQuery] = useState("");
  const [skillQuery, setSkillQuery] = useState("");

  const [confirmDelete, setConfirmDelete] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const [refreshingSkills, setRefreshingSkills] = useState(false);
  const [uploadingSkill, setUploadingSkill] = useState(false);
  const [skillMenuOpen, setSkillMenuOpen] = useState(false);

  const reloadSkillsList = useCallback(() => {
    api.listSkills(activeAgentId).then(setAgentSkills).catch(() => {});
    api.getSkillsDenyList().then(setSkillsDeny).catch(() => {});
  }, [activeAgentId]);

  const loadModels = useCallback(() => {
    if (!gatewayReady) return;
    api.listModels().then(setModels).catch(() => {});
  }, [gatewayReady]);

  const [backendAgent, setBackendAgent] = useState<api.BackendAgent | null>(null);

  useEffect(() => {
    if (!gatewayReady) return;
    Promise.all([
      api.listModels().catch(() => [] as api.ModelInfo[]),
      api.listAgentTools(activeAgentId).catch(() => [] as api.AgentToolInfo[]),
      api.listSkills(activeAgentId).catch(() => [] as api.SkillInfo[]),
      api.getSkillsDenyList().catch(() => [] as string[]),
      api.getAgent(activeAgentId).catch(() => null),
    ]).then(([m, tools, skills, deny, a]) => {
      setModels(m);
      setAgentTools(tools);
      setAgentSkills(skills);
      setSkillsDeny(deny);
      if (a) {
        setBackendAgent(a);
        if (typeof a.model === "string") {
          setSelectedModel(a.model);
          setSelectedProvider(null);
        } else if (a.model) {
          setSelectedModel(a.model.model);
          setSelectedProvider(a.model.provider);
        }
        setFileAccessMode(a.behavior?.fileAccess ?? "workspace");
      }
    });
  }, [activeAgentId, gatewayReady]);

  useEffect(() => {
    const onModelsUpdated = () => loadModels();
    window.addEventListener("fastclaw:models-updated", onModelsUpdated);
    return () => window.removeEventListener("fastclaw:models-updated", onModelsUpdated);
  }, [loadModels]);

  const handleSave = useCallback(async () => {
    setSaving(true);
    setSaveMsg("");
    const currentModel = backendAgent?.model;
    const currentModelObj =
      currentModel && typeof currentModel === "object" ? currentModel : null;
    const selectedModelMeta = models.find((m) =>
      m.model === selectedModel && (!selectedProvider || m.provider === selectedProvider),
    );
    const modelConfig: api.AgentModelConfig = {
      provider:
        selectedProvider ??
        selectedModelMeta?.provider ??
        currentModelObj?.provider ??
        "openai",
      model: selectedModel,
      temperature: currentModelObj?.temperature ?? 0,
      maxTokens: currentModelObj?.maxTokens,
      contextWindow: currentModelObj?.contextWindow,
      costPer1kInput: currentModelObj?.costPer1kInput,
      costPer1kOutput: currentModelObj?.costPer1kOutput,
      supportsReasoning: currentModelObj?.supportsReasoning,
      fallbacks: currentModelObj?.fallbacks,
      maxConcurrentRequests: currentModelObj?.maxConcurrentRequests,
    };
    const payload: api.BackendAgent = {
      agentId: activeAgentId,
      ...(backendAgent ?? {}),
      name: name || activeAgentId,
      model: modelConfig,
      behavior: {
        ...(backendAgent?.behavior ?? {}),
        fileAccess: fileAccessMode,
      },
    };
    const ok = await api.updateAgent(activeAgentId, payload);
    if (ok && !backendAgent) {
      const refreshed = await api.getAgent(activeAgentId).catch(() => null);
      if (refreshed) setBackendAgent(refreshed);
    }
    setSaving(false);
    setSaveMsg(ok ? "已保存" : "保存失败");
    setTimeout(() => setSaveMsg(""), 2000);
  }, [activeAgentId, name, selectedModel, selectedProvider, backendAgent, fileAccessMode, models]);

  const handleToolToggle = useCallback(async (toolId: string, newEnabled: boolean) => {
    setTogglingTool(toolId);
    setAgentTools((prev) => prev.map((t) => t.id === toolId ? { ...t, enabled: newEnabled } : t));
    const snapshot = agentTools;
    const updated = agentTools.map((t) => ({ id: t.id, enabled: t.id === toolId ? newEnabled : t.enabled }));
    const ok = await api.updateAgentTools(activeAgentId, updated);
    if (!ok) setAgentTools(snapshot);
    setTogglingTool(null);
  }, [activeAgentId, agentTools]);

  const handleSkillToggle = useCallback(async (skillId: string, newEnabled: boolean) => {
    setTogglingSkill(skillId);
    setSkillsDeny((prev) => newEnabled ? prev.filter((id) => id !== skillId) : prev.includes(skillId) ? prev : [...prev, skillId]);
    const prevDeny = skillsDeny;
    const newDeny = newEnabled ? skillsDeny.filter((id) => id !== skillId) : [...skillsDeny, skillId];
    const ok = await api.updateSkillsDenyList(newDeny);
    if (!ok) setSkillsDeny(prevDeny);
    setTogglingSkill(null);
  }, [skillsDeny]);

  const handleDelete = useCallback(async () => {
    setDeleting(true);
    const ok = await api.deleteAgent(activeAgentId);
    if (ok) {
      removeAgent(activeAgentId);
    } else {
      setSaveMsg("删除失败");
      setTimeout(() => setSaveMsg(""), 2000);
    }
    setDeleting(false);
    setConfirmDelete(false);
  }, [activeAgentId, removeAgent]);

  const handleRefreshSkills = useCallback(async () => {
    setRefreshingSkills(true);
    await api.refreshSkills();
    reloadSkillsList();
    setRefreshingSkills(false);
  }, [reloadSkillsList]);

  const handleUploadSkillZip = useCallback(async () => {
    setUploadingSkill(true);
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const selected = await open({
        title: "选择 Skill ZIP 文件",
        directory: false,
        multiple: false,
        filters: [{ name: "ZIP", extensions: ["zip"] }],
      });
      if (selected) {
        await api.uploadSkill(selected as string);
        await api.refreshSkills();
        reloadSkillsList();
      }
    } catch { /* user cancelled */ }
    setUploadingSkill(false);
  }, [reloadSkillsList]);

  const handleUploadSkillFolder = useCallback(async () => {
    setUploadingSkill(true);
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const selected = await open({
        title: "选择 Skill 文件夹（需包含 SKILL.md）",
        directory: true,
        multiple: false,
      });
      if (selected) {
        await api.uploadSkill(selected as string);
        await api.refreshSkills();
        reloadSkillsList();
      }
    } catch { /* user cancelled */ }
    setUploadingSkill(false);
  }, [reloadSkillsList]);

  const nonMcpTools = useMemo(() => agentTools.filter((t) => !t.name.startsWith("mcp_")), [agentTools]);
  const filteredTools = useMemo(() => {
    if (!toolQuery) return nonMcpTools;
    const q = toolQuery.toLowerCase();
    return nonMcpTools.filter((t) => t.name.toLowerCase().includes(q) || t.description?.toLowerCase().includes(q));
  }, [nonMcpTools, toolQuery]);

  const filteredSkills = useMemo(() => {
    if (!skillQuery) return agentSkills;
    const q = skillQuery.toLowerCase();
    return agentSkills.filter((s) => s.name.toLowerCase().includes(q) || s.description?.toLowerCase().includes(q));
  }, [agentSkills, skillQuery]);

  if (!agent) return null;

  const effectiveModel = (typeof backendAgent?.model === "string" ? backendAgent.model : backendAgent?.model?.model) ?? agent.model;
  const effectiveProvider = typeof backendAgent?.model === "object" ? backendAgent.model.provider : (selectedProvider ?? "");
  const selectedModelValue = selectedProvider ? encodeModelOption(selectedProvider, selectedModel) : selectedModel;
  const effectiveOptionValue = effectiveProvider ? encodeModelOption(effectiveProvider, effectiveModel) : effectiveModel;
  const isLastAgent = agents.length <= 1;

  return (
    <div className="space-y-5 p-4">
      {/* Name */}
      <div>
        <SectionHeader>名称</SectionHeader>
        <input
          type="text"
          value={name}
          onChange={(e) => setName(e.target.value)}
          className="w-full rounded-[var(--radius-sm)] px-3 py-2 text-[13px] outline-none transition-colors duration-150 focus:ring-1 focus:ring-[var(--fill-quaternary)]"
          style={{ background: "var(--bg-elevated)", color: "var(--fill-primary)", border: "0.5px solid var(--separator-opaque)" }}
        />
      </div>

      {/* Model */}
      <div>
        <SectionHeader>模型</SectionHeader>
        <div className="relative">
          <select
            value={selectedModelValue}
            onChange={(e) => {
              const parsed = decodeModelOption(e.target.value);
              setSelectedModel(parsed.model);
              setSelectedProvider(parsed.provider);
            }}
            className="w-full cursor-pointer rounded-[var(--radius-sm)] px-3 py-2.5 pr-8 text-[13px] outline-none transition-colors duration-150 focus:ring-1 focus:ring-[var(--fill-quaternary)]"
            style={{ background: "var(--bg-elevated)", color: "var(--fill-primary)", border: "0.5px solid var(--separator-opaque)", WebkitAppearance: "none", MozAppearance: "none", appearance: "none" }}
          >
            {models.map((m) => (
              <option key={`${m.provider}/${m.model}`} value={encodeModelOption(m.provider, m.model)}>{m.model} ({m.provider})</option>
            ))}
            {!models.some((m) => encodeModelOption(m.provider, m.model) === effectiveOptionValue) && (
              <option value={effectiveOptionValue}>
                {effectiveProvider ? `${effectiveModel} (${effectiveProvider})` : effectiveModel}
              </option>
            )}
          </select>
          <ChevronDown size={12} strokeWidth={2} className="pointer-events-none absolute top-1/2 right-3 -translate-y-1/2" style={{ color: "var(--fill-tertiary)" }} />
        </div>
      </div>

      {/* Identity Files */}
      <IdentitySection agentId={activeAgentId} ready={gatewayReady} />

      {/* Tools */}
      <div>
        <SectionHeader>文件访问权限</SectionHeader>
        <div className="relative">
          <select
            value={fileAccessMode}
            onChange={(e) => setFileAccessMode(e.target.value as api.FileAccessMode)}
            className="w-full cursor-pointer rounded-[var(--radius-sm)] px-3 py-2.5 pr-8 text-[13px] outline-none transition-colors duration-150 focus:ring-1 focus:ring-[var(--fill-quaternary)]"
            style={{ background: "var(--bg-elevated)", color: "var(--fill-primary)", border: "0.5px solid var(--separator-opaque)", WebkitAppearance: "none", MozAppearance: "none", appearance: "none" }}
          >
            <option value="none">禁止访问文件系统</option>
            <option value="workspace">仅访问工作区</option>
            <option value="full">完全访问文件系统</option>
          </select>
          <ChevronDown size={12} strokeWidth={2} className="pointer-events-none absolute top-1/2 right-3 -translate-y-1/2" style={{ color: "var(--fill-tertiary)" }} />
        </div>
        <p className="mt-1.5 text-[11px]" style={{ color: "var(--fill-quaternary)" }}>
          控制 read_file、write_file、edit_file、apply_patch、search_in_files 等文件工具的访问范围。
        </p>
      </div>

      {/* Tools */}
      <div>
        <SectionHeader count={nonMcpTools.filter((t) => t.enabled).length} total={nonMcpTools.length} searchable query={toolQuery} onQueryChange={setToolQuery}>
          工具
        </SectionHeader>
        <CollapsibleList
          items={filteredTools}
          emptyText={toolQuery ? "无匹配工具" : "未获取到工具列表"}
          renderItem={(tool, _i, isLast) => (
            <div
              key={tool.id}
              className="flex items-center justify-between gap-2 px-3 py-2.5 transition-colors duration-100 hover:bg-[var(--bg-hover)]"
              style={{ borderBottom: isLast ? "none" : "0.5px solid var(--separator)", opacity: tool.enabled ? 1 : 0.55 }}
            >
              <div className="min-w-0 flex-1">
                <span className="block truncate text-[13px]" style={{ color: "var(--fill-primary)" }} title={tool.name}>{tool.name}</span>
                {tool.description && <div className="mt-0.5 truncate text-[11px]" style={{ color: "var(--fill-tertiary)" }} title={tool.description}>{tool.description}</div>}
              </div>
              <Toggle checked={tool.enabled} onChange={(v) => handleToolToggle(tool.id, v)} disabled={togglingTool === tool.id} />
            </div>
          )}
        />
      </div>

      {/* Skills */}
      <div>
        <div className="flex items-center justify-between">
          <SectionHeader count={agentSkills.filter((s) => !skillsDeny.includes(s.id)).length} total={agentSkills.length} searchable query={skillQuery} onQueryChange={setSkillQuery}>
            Skills
          </SectionHeader>
          <div className="flex items-center gap-1">
            <button
              onClick={handleRefreshSkills}
              disabled={refreshingSkills}
              className="cursor-pointer rounded-[var(--radius-xs)] p-1.5 transition-colors duration-100 hover:bg-[var(--bg-hover)] disabled:opacity-40"
              title="刷新 Skills"
            >
              <RefreshCw size={13} strokeWidth={1.5} className={refreshingSkills ? "animate-spin" : ""} style={{ color: "var(--fill-tertiary)" }} />
            </button>
            <div className="relative">
              <button
                onClick={() => setSkillMenuOpen((v) => !v)}
                disabled={uploadingSkill}
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
                    onClick={() => { setSkillMenuOpen(false); handleUploadSkillFolder(); }}
                    className="w-full cursor-pointer px-3 py-2 text-left text-[12px] transition-colors hover:bg-[var(--bg-hover)]"
                    style={{ color: "var(--fill-primary)" }}
                  >
                    <FolderOpen size={12} className="mr-2 inline" strokeWidth={1.5} />选择文件夹
                  </button>
                  <button
                    onClick={() => { setSkillMenuOpen(false); handleUploadSkillZip(); }}
                    className="w-full cursor-pointer px-3 py-2 text-left text-[12px] transition-colors hover:bg-[var(--bg-hover)]"
                    style={{ color: "var(--fill-primary)" }}
                  >
                    <FileText size={12} className="mr-2 inline" strokeWidth={1.5} />选择 ZIP 文件
                  </button>
                </div>
              )}
            </div>
          </div>
        </div>
        <CollapsibleList
          items={filteredSkills}
          emptyText={skillQuery ? "无匹配技能" : "未获取到 Skills"}
          renderItem={(skill, _i, isLast) => {
            const enabled = !skillsDeny.includes(skill.id);
            return (
              <div
                key={skill.id}
                className="flex items-center justify-between gap-2 px-3 py-2.5 transition-colors duration-100 hover:bg-[var(--bg-hover)]"
                style={{ borderBottom: isLast ? "none" : "0.5px solid var(--separator)", opacity: enabled ? 1 : 0.55 }}
              >
                <div className="min-w-0 flex-1">
                  <div className="flex min-w-0 items-center gap-2">
                    <span className="min-w-0 truncate text-[13px]" style={{ color: "var(--fill-primary)" }} title={skill.name}>{skill.name}</span>
                    {skill.version && <span className="shrink-0 text-[10px]" style={{ color: "var(--fill-quaternary)" }}>v{skill.version}</span>}
                  </div>
                  {skill.description && <div className="mt-0.5 line-clamp-2 text-[11px]" style={{ color: "var(--fill-tertiary)" }} title={skill.description}>{skill.description}</div>}
                </div>
                <Toggle checked={enabled} onChange={(v) => handleSkillToggle(skill.id, v)} disabled={togglingSkill === skill.id} />
              </div>
            );
          }}
        />
      </div>

      {/* Channels */}
      <ChannelManager agentId={activeAgentId} backendAgent={backendAgent} ready={gatewayReady} />

      {/* Save */}
      <div className="flex items-center gap-3 pt-2">
        <button
          onClick={handleSave}
          disabled={saving}
          className="cursor-pointer rounded-[var(--radius-sm)] px-5 py-2 text-[13px] font-medium transition-opacity duration-150 hover:opacity-90 disabled:opacity-50"
          style={{ background: "var(--fill-primary)", color: "var(--fill-inverse)" }}
        >
          {saving ? "保存中..." : "保存配置"}
        </button>
        {saveMsg && <span className="text-[12px]" style={{ color: "var(--fill-tertiary)" }}>{saveMsg}</span>}
      </div>

      {/* Delete Agent */}
      <div className="pt-4" style={{ borderTop: "0.5px solid var(--separator)" }}>
        <SectionHeader>危险操作</SectionHeader>
        {confirmDelete ? (
          <ListContainer>
            <div className="flex items-center gap-3 px-3 py-3">
              <AlertTriangle size={14} strokeWidth={1.5} className="shrink-0" style={{ color: "var(--fill-tertiary)" }} />
              <span className="flex-1 text-[12px]" style={{ color: "var(--fill-secondary)" }}>
                确认删除 "{agent.name}"？此操作不可撤销。
              </span>
            </div>
            <div className="flex gap-2 px-3 pb-3">
              <button
                onClick={handleDelete}
                disabled={deleting}
                className="cursor-pointer rounded-[var(--radius-xs)] px-4 py-1.5 text-[12px] font-medium transition-opacity hover:opacity-80 disabled:opacity-50"
                style={{ background: "var(--fill-tertiary)", color: "var(--fill-inverse)" }}
              >
                {deleting ? "删除中..." : "确认删除"}
              </button>
              <button
                onClick={() => setConfirmDelete(false)}
                className="cursor-pointer rounded-[var(--radius-xs)] px-4 py-1.5 text-[12px] transition-colors duration-100 hover:bg-[var(--bg-hover)]"
                style={{ color: "var(--fill-secondary)" }}
              >
                取消
              </button>
            </div>
          </ListContainer>
        ) : (
          <button
            onClick={() => setConfirmDelete(true)}
            disabled={isLastAgent}
            className="cursor-pointer text-[12px] transition-colors duration-100 hover:opacity-70 disabled:cursor-not-allowed disabled:opacity-40"
            style={{ color: "var(--fill-tertiary)" }}
            title={isLastAgent ? "至少保留一个 Agent" : undefined}
          >
            <span className="flex items-center gap-1.5">
              <Trash2 size={12} strokeWidth={1.5} />
              删除此 Agent
            </span>
          </button>
        )}
      </div>
    </div>
  );
}

/* ━━━ Chat Row ━━━ */

function ChatRow({ chat, isActive, onClick, onClose, isLast }: {
  chat: import("../../lib/agent-store").Chat;
  isActive: boolean;
  onClick: () => void;
  onClose?: () => void;
  isLast: boolean;
}) {
  return (
    <div
      className="group relative flex w-full cursor-pointer flex-col gap-1 px-3 py-2.5 text-left transition-colors duration-100 hover:bg-[var(--bg-hover)]"
      style={{
        background: isActive ? "var(--tint-bg)" : "transparent",
        borderBottom: isLast ? "none" : "0.5px solid var(--separator)",
      }}
      onClick={onClick}
    >
      {onClose && (
        <button
          onClick={(e) => { e.stopPropagation(); onClose(); }}
          className="absolute top-2 right-2 flex h-5 w-5 items-center justify-center rounded-md opacity-0 transition-opacity duration-100 group-hover:opacity-100"
          style={{ color: "var(--fill-quaternary)" }}
          title="关闭会话"
        >
          <X size={8} strokeWidth={2.5} />
        </button>
      )}
      <div className="flex items-start justify-between gap-2">
        <span className="min-w-0 flex-1 truncate text-[13px] font-medium leading-tight" style={{ color: "var(--fill-primary)" }} title={chat.title}>
          {chat.title}
        </span>
        {isActive && (
          <span className="mt-0.5 shrink-0 rounded-full px-1.5 py-0.5 text-[10px] font-medium" style={{ background: "var(--fill-primary)", color: "var(--fill-inverse)" }}>当前</span>
        )}
      </div>
      {chat.workDir && (
        <div className="flex items-center gap-1.5 text-[10px] font-mono" style={{ color: "var(--fill-quaternary)" }}>
          <FolderOpen size={10} strokeWidth={1.5} />
          <span className="truncate">{chat.workDir.replace(/^\/home\/[^/]+\//, "~/")}</span>
        </div>
      )}
      <div className="flex items-center gap-2 text-[11px]" style={{ color: "var(--fill-tertiary)" }}>
        <span>{chat.createdAt.toLocaleDateString("zh-CN", { month: "numeric", day: "numeric", hour: "2-digit", minute: "2-digit" })}</span>
        <span>·</span>
        <span>{chat.messageCount} 条消息</span>
      </div>
    </div>
  );
}

/* ━━━ Chats Tab ━━━ */

function ChatsTab() {
  const activeAgentId = useAgentStore((s) => s.activeAgentId);
  const agentChats = useAgentStore((s) => s.agentChats);
  const setActiveChat = useAgentStore((s) => s.setActiveChat);
  const reopenChat = useAgentStore((s) => s.reopenChat);
  const closeChat = useAgentStore((s) => s.closeChat);
  const ac = agentChats[activeAgentId];

  const [chatQuery, setChatQuery] = useState("");

  if (!ac) return null;

  const openChats = ac.chatList.filter((c) => c.open);
  const closedChats = ac.chatList.filter((c) => !c.open);

  const filteredOpen = chatQuery
    ? openChats.filter((c) => c.title.toLowerCase().includes(chatQuery.toLowerCase()))
    : openChats;
  const filteredClosed = chatQuery
    ? closedChats.filter((c) => c.title.toLowerCase().includes(chatQuery.toLowerCase()))
    : closedChats;

  return (
    <div className="p-4">
      {/* Search */}
      <div
        className="mb-4 flex items-center gap-2.5 rounded-[10px] px-3 py-[7px]"
        style={{ background: "var(--bg-hover)" }}
      >
        <Search size={12} strokeWidth={1.5} style={{ color: "var(--fill-tertiary)" }} />
        <input
          type="text"
          value={chatQuery}
          onChange={(e) => setChatQuery(e.target.value)}
          placeholder="搜索会话..."
          className="min-w-0 flex-1 bg-transparent text-[12px] outline-none"
          style={{ color: "var(--fill-primary)" }}
        />
        {chatQuery && (
          <button onClick={() => setChatQuery("")} className="cursor-pointer" style={{ color: "var(--fill-quaternary)" }}>
            <X size={10} strokeWidth={2} />
          </button>
        )}
      </div>

      {filteredOpen.length > 0 && (
        <div className="mb-4">
          <div className="mb-1.5 flex items-center gap-1.5 px-1">
            <span className="text-[11px] font-medium uppercase tracking-wider" style={{ color: "var(--fill-tertiary)" }}>已打开</span>
            <span className="text-[10px]" style={{ color: "var(--fill-quaternary)" }}>({filteredOpen.length})</span>
          </div>
          <ListContainer>
            {filteredOpen.map((chat, i) => (
              <ChatRow
                key={chat.id}
                chat={chat}
                isActive={chat.id === ac.activeChatId}
                onClick={() => setActiveChat(activeAgentId, chat.id)}
                onClose={() => closeChat(activeAgentId, chat.id)}
                isLast={i === filteredOpen.length - 1}
              />
            ))}
          </ListContainer>
        </div>
      )}

      {filteredClosed.length > 0 && (
        <div>
          <div className="mb-1.5 flex items-center gap-1.5 px-1">
            <span className="text-[11px] font-medium uppercase tracking-wider" style={{ color: "var(--fill-tertiary)" }}>历史会话</span>
            <span className="text-[10px]" style={{ color: "var(--fill-quaternary)" }}>({filteredClosed.length})</span>
          </div>
          <ListContainer>
            {filteredClosed.map((chat, i) => (
              <ChatRow
                key={chat.id}
                chat={chat}
                isActive={false}
                onClick={() => reopenChat(activeAgentId, chat.id)}
                isLast={i === filteredClosed.length - 1}
              />
            ))}
          </ListContainer>
        </div>
      )}

      {ac.chatList.length === 0 && (
        <div className="py-12 text-center">
          <p className="text-[13px]" style={{ color: "var(--fill-tertiary)" }}>暂无会话</p>
        </div>
      )}
    </div>
  );
}

/* ━━━ Cron Tab ━━━ */

/* ━━━ Schedule Picker helpers ━━━ */

type ScheduleMode = "every_n_min" | "every_n_hour" | "daily" | "weekly" | "custom";

function scheduleToMode(schedule: string): { mode: ScheduleMode; minutes?: number; hours?: number; atHour?: number; atMin?: number; weekdays?: number[] } {
  const parts = schedule.trim().split(/\s+/);
  if (parts.length !== 6) return { mode: "custom" };
  const [sec, min, hour, _dom, _mon, dow] = parts;
  if (sec !== "0") return { mode: "custom" };
  const mInterval = min.match(/^\*\/(\d+)$/);
  if (mInterval && hour === "*" && dow === "*") return { mode: "every_n_min", minutes: parseInt(mInterval[1]) };
  const hInterval = hour.match(/^\*\/(\d+)$/);
  if (min === "0" && hInterval && dow === "*") return { mode: "every_n_hour", hours: parseInt(hInterval[1]) };
  if (/^\d+$/.test(min) && /^\d+$/.test(hour) && dow === "*") return { mode: "daily", atHour: parseInt(hour), atMin: parseInt(min) };
  if (/^\d+$/.test(min) && /^\d+$/.test(hour) && /^[\d,\-]+$/.test(dow)) {
    const days = dow.split(",").flatMap(s => { const r = s.split("-"); return r.length === 2 ? Array.from({ length: parseInt(r[1]) - parseInt(r[0]) + 1 }, (_, i) => parseInt(r[0]) + i) : [parseInt(s)]; });
    return { mode: "weekly", atHour: parseInt(hour), atMin: parseInt(min), weekdays: days };
  }
  return { mode: "custom" };
}

function modeToSchedule(mode: ScheduleMode, opts: { minutes?: number; hours?: number; atHour?: number; atMin?: number; weekdays?: number[]; custom?: string }): string {
  switch (mode) {
    case "every_n_min": return `0 */${opts.minutes ?? 5} * * * *`;
    case "every_n_hour": return `0 0 */${opts.hours ?? 1} * * *`;
    case "daily": return `0 ${opts.atMin ?? 0} ${opts.atHour ?? 9} * * *`;
    case "weekly": return `0 ${opts.atMin ?? 0} ${opts.atHour ?? 9} * * ${(opts.weekdays ?? [1, 2, 3, 4, 5]).join(",")}`;
    case "custom": return opts.custom ?? "0 */5 * * * *";
  }
}

const WEEKDAY_NAMES = ["日", "一", "二", "三", "四", "五", "六"];

function SchedulePicker({ schedule, onChange }: { schedule: string; onChange: (s: string) => void }) {
  const parsed = useMemo(() => scheduleToMode(schedule), [schedule]);
  const [mode, setMode] = useState<ScheduleMode>(parsed.mode);
  const [minutes, setMinutes] = useState(parsed.minutes ?? 5);
  const [hours, setHours] = useState(parsed.hours ?? 1);
  const [atHour, setAtHour] = useState(parsed.atHour ?? 9);
  const [atMin, setAtMin] = useState(parsed.atMin ?? 0);
  const [weekdays, setWeekdays] = useState<number[]>(parsed.weekdays ?? [1, 2, 3, 4, 5]);
  const [custom, setCustom] = useState(schedule);

  const emit = useCallback((m: ScheduleMode, o: Parameters<typeof modeToSchedule>[1]) => {
    onChange(modeToSchedule(m, o));
  }, [onChange]);

  const selectCls = "w-full rounded-[6px] px-3 py-2 text-[13px] outline-none cursor-pointer pr-8 transition-colors focus:ring-1 focus:ring-[var(--fill-quaternary)]";
  const selectStyle = { background: "var(--bg-base)", color: "var(--fill-primary)", border: "0.5px solid var(--separator-opaque)", WebkitAppearance: "none" as const, appearance: "none" as const };
  const inlineCls = "rounded-[6px] px-2.5 py-1.5 text-[13px] outline-none text-center transition-colors focus:ring-1 focus:ring-[var(--fill-quaternary)]";
  const inlineStyle = { background: "var(--bg-base)", color: "var(--fill-primary)", border: "0.5px solid var(--separator-opaque)" };
  const labelStyle = { color: "var(--fill-tertiary)" };

  return (
    <div className="space-y-2">
      <label className="mb-1 block text-[11px] font-medium" style={labelStyle}>执行频率</label>
      <div className="relative">
        <select value={mode} onChange={(e) => { const m = e.target.value as ScheduleMode; setMode(m); emit(m, { minutes, hours, atHour, atMin, weekdays, custom }); }} className={selectCls} style={selectStyle}>
          <option value="every_n_min">每隔 N 分钟</option>
          <option value="every_n_hour">每隔 N 小时</option>
          <option value="daily">每天定时</option>
          <option value="weekly">每周定时</option>
          <option value="custom">自定义 Cron</option>
        </select>
        <ChevronDown size={10} strokeWidth={2} className="pointer-events-none absolute top-1/2 right-3 -translate-y-1/2" style={labelStyle} />
      </div>

      {mode === "every_n_min" && (
        <div className="flex items-center gap-2 text-[13px]" style={{ color: "var(--fill-secondary)" }}>
          <span>每</span>
          <input type="number" min={1} max={59} value={minutes} onChange={(e) => { const v = Math.max(1, Math.min(59, parseInt(e.target.value) || 1)); setMinutes(v); emit(mode, { minutes: v }); }} className={inlineCls + " w-16"} style={inlineStyle} />
          <span>分钟</span>
        </div>
      )}
      {mode === "every_n_hour" && (
        <div className="flex items-center gap-2 text-[13px]" style={{ color: "var(--fill-secondary)" }}>
          <span>每</span>
          <input type="number" min={1} max={23} value={hours} onChange={(e) => { const v = Math.max(1, Math.min(23, parseInt(e.target.value) || 1)); setHours(v); emit(mode, { hours: v }); }} className={inlineCls + " w-16"} style={inlineStyle} />
          <span>小时</span>
        </div>
      )}
      {(mode === "daily" || mode === "weekly") && (
        <div className="flex items-center gap-2 text-[13px]" style={{ color: "var(--fill-secondary)" }}>
          <span>时间</span>
          <input type="number" min={0} max={23} value={atHour} onChange={(e) => { const v = Math.max(0, Math.min(23, parseInt(e.target.value) || 0)); setAtHour(v); emit(mode, { atHour: v, atMin, weekdays }); }} className={inlineCls + " w-14"} style={inlineStyle} />
          <span>:</span>
          <input type="number" min={0} max={59} value={atMin} onChange={(e) => { const v = Math.max(0, Math.min(59, parseInt(e.target.value) || 0)); setAtMin(v); emit(mode, { atHour, atMin: v, weekdays }); }} className={inlineCls + " w-14"} style={inlineStyle} />
        </div>
      )}
      {mode === "weekly" && (
        <div className="flex flex-wrap gap-1">
          {WEEKDAY_NAMES.map((name, i) => (
            <button key={i} onClick={() => { const next = weekdays.includes(i) ? weekdays.filter(d => d !== i) : [...weekdays, i].sort(); setWeekdays(next); emit(mode, { atHour, atMin, weekdays: next }); }}
              className="cursor-pointer rounded-[4px] px-2 py-1 text-[11px] font-medium transition-colors"
              style={{ background: weekdays.includes(i) ? "var(--fill-primary)" : "var(--bg-base)", color: weekdays.includes(i) ? "var(--fill-inverse)" : "var(--fill-tertiary)", border: "0.5px solid var(--separator-opaque)" }}
            >{name}</button>
          ))}
        </div>
      )}
      {mode === "custom" && (
        <div>
          <input value={custom} onChange={(e) => { setCustom(e.target.value); onChange(e.target.value); }} placeholder="0 */5 * * * *" className={"w-full rounded-[6px] px-3 py-2 font-mono text-[13px] outline-none transition-colors focus:ring-1 focus:ring-[var(--fill-quaternary)]"} style={inlineStyle} />
          <p className="mt-1 text-[10px]" style={{ color: "var(--fill-quaternary)" }}>6 字段: 秒 分 时 日 月 周</p>
        </div>
      )}
    </div>
  );
}

/* ━━━ Run Log viewer ━━━ */

function RunLogList({ jobId }: { jobId: string }) {
  const [runs, setRuns] = useState<CronJobRun[]>([]);
  const [loading, setLoading] = useState(false);
  const [expanded, setExpanded] = useState<number | null>(null);

  useEffect(() => {
    setLoading(true);
    api.listCronRuns(jobId, 20).then(setRuns).finally(() => setLoading(false));
  }, [jobId]);

  if (loading) return <p className="py-3 text-center text-[11px]" style={{ color: "var(--fill-quaternary)" }}>加载中...</p>;
  if (runs.length === 0) return <p className="py-3 text-center text-[11px]" style={{ color: "var(--fill-quaternary)" }}>暂无执行记录</p>;

  return (
    <div className="space-y-1">
      {runs.map((run) => (
        <div key={run.id} className="rounded-[6px] text-[11px]" style={{ background: "var(--bg-base)", border: "0.5px solid var(--separator-opaque)" }}>
          <div className="flex cursor-pointer items-center justify-between gap-2 px-2.5 py-1.5" onClick={() => setExpanded(expanded === run.id ? null : run.id)}>
            <div className="flex items-center gap-2">
              <span className="inline-block h-[6px] w-[6px] rounded-full" style={{ background: run.status === "ok" ? "var(--green, #48bb78)" : run.status === "running" ? "var(--blue, #4299e1)" : "var(--red, #e53e3e)" }} />
              <span style={{ color: "var(--fill-secondary)" }}>{new Date(run.started_at).toLocaleString("zh-CN")}</span>
            </div>
            <span style={{ color: "var(--fill-quaternary)" }}>{run.status === "ok" ? "成功" : run.status === "running" ? "运行中" : "失败"}</span>
          </div>
          {expanded === run.id && (
            <div className="border-t px-2.5 py-2" style={{ borderColor: "var(--separator-opaque)" }}>
              {run.output && (
                <div className="mb-1">
                  <span className="font-medium" style={{ color: "var(--fill-tertiary)" }}>Agent 回复:</span>
                  <pre className="mt-1 max-h-40 overflow-auto whitespace-pre-wrap rounded-[4px] p-2 text-[11px]" style={{ background: "var(--bg-tertiary)", color: "var(--fill-secondary)" }}>{run.output}</pre>
                </div>
              )}
              {run.error && (
                <div>
                  <span className="font-medium" style={{ color: "var(--red, #e53e3e)" }}>错误:</span>
                  <pre className="mt-1 max-h-24 overflow-auto whitespace-pre-wrap rounded-[4px] p-2 text-[11px]" style={{ background: "var(--bg-tertiary)", color: "var(--red, #e53e3e)" }}>{run.error}</pre>
                </div>
              )}
              {run.ended_at && (
                <p className="mt-1" style={{ color: "var(--fill-quaternary)" }}>
                  耗时: {Math.round((new Date(run.ended_at).getTime() - new Date(run.started_at).getTime()) / 1000)}s
                </p>
              )}
            </div>
          )}
        </div>
      ))}
    </div>
  );
}

/* ━━━ Cron Job Form ━━━ */

const EMPTY_CRON_JOB: Partial<CronJob> & { schedule: string; action: CronJobAction } = {
  name: "",
  schedule: "0 */5 * * * *",
  action: { type: "agent_chat", agent_id: "", message: "" },
  enabled: true,
};

function CronJobForm({
  job,
  agentId,
  isNew,
  onSave,
  onCancel,
  onDelete,
  saving,
}: {
  job: Partial<CronJob> & { schedule: string; action: CronJobAction };
  agentId: string;
  isNew: boolean;
  onSave: (j: Partial<CronJob> & { schedule: string; action: CronJobAction }) => void;
  onCancel: () => void;
  onDelete?: () => void;
  saving: boolean;
}) {
  const [form, setForm] = useState({ ...job });
  const [actionType, setActionType] = useState<"agent_chat" | "webhook">(
    job.action?.type === "webhook" ? "webhook" : "agent_chat",
  );
  const [showLogs, setShowLogs] = useState(!isNew);

  const inputCls = "w-full rounded-[6px] px-3 py-2 text-[13px] outline-none transition-colors focus:ring-1 focus:ring-[var(--fill-quaternary)]";
  const inputStyle = { background: "var(--bg-base)", color: "var(--fill-primary)", border: "0.5px solid var(--separator-opaque)" };
  const labelCls = "mb-1 block text-[11px] font-medium";
  const labelStyle = { color: "var(--fill-tertiary)" };

  const handleSubmit = () => {
    const action: CronJobAction = actionType === "webhook"
      ? { type: "webhook", url: form.action?.url ?? "", method: form.action?.method ?? "POST", body: form.action?.body }
      : { type: "agent_chat", agent_id: agentId, message: form.action?.message ?? "" };
    onSave({ ...form, action });
  };

  return (
    <div className="space-y-3">
      <div>
        <label className={labelCls} style={labelStyle}>任务名称</label>
        <input
          value={form.name ?? ""}
          onChange={(e) => setForm((f) => ({ ...f, name: e.target.value }))}
          placeholder="每日汇报"
          className={inputCls}
          style={inputStyle}
        />
      </div>

      <SchedulePicker schedule={form.schedule} onChange={(s) => setForm((f) => ({ ...f, schedule: s }))} />

      <div>
        <label className={labelCls} style={labelStyle}>动作类型</label>
        <div className="relative">
          <select
            value={actionType}
            onChange={(e) => setActionType(e.target.value as "agent_chat" | "webhook")}
            className={inputCls + " cursor-pointer pr-8"}
            style={{ ...inputStyle, WebkitAppearance: "none", appearance: "none" } as React.CSSProperties}
          >
            <option value="agent_chat">Agent 对话</option>
            <option value="webhook">Webhook</option>
          </select>
          <ChevronDown size={10} strokeWidth={2} className="pointer-events-none absolute top-1/2 right-3 -translate-y-1/2" style={labelStyle} />
        </div>
      </div>

      {actionType === "agent_chat" ? (
        <div>
          <label className={labelCls} style={labelStyle}>消息内容</label>
          <textarea
            value={form.action?.message ?? ""}
            onChange={(e) =>
              setForm((f) => ({ ...f, action: { ...f.action, message: e.target.value } }))
            }
            placeholder="请生成今日工作汇报"
            rows={3}
            className={inputCls + " resize-none"}
            style={inputStyle}
          />
        </div>
      ) : (
        <>
          <div>
            <label className={labelCls} style={labelStyle}>Webhook URL</label>
            <input
              value={form.action?.url ?? ""}
              onChange={(e) =>
                setForm((f) => ({ ...f, action: { ...f.action, url: e.target.value } }))
              }
              placeholder="https://example.com/webhook"
              className={inputCls + " font-mono"}
              style={inputStyle}
            />
          </div>
          <div>
            <label className={labelCls} style={labelStyle}>HTTP 方法</label>
            <div className="relative">
              <select
                value={form.action?.method ?? "POST"}
                onChange={(e) =>
                  setForm((f) => ({ ...f, action: { ...f.action, method: e.target.value } }))
                }
                className={inputCls + " cursor-pointer pr-8"}
                style={{ ...inputStyle, WebkitAppearance: "none", appearance: "none" } as React.CSSProperties}
              >
                <option value="POST">POST</option>
                <option value="GET">GET</option>
                <option value="PUT">PUT</option>
                <option value="DELETE">DELETE</option>
              </select>
              <ChevronDown size={10} strokeWidth={2} className="pointer-events-none absolute top-1/2 right-3 -translate-y-1/2" style={labelStyle} />
            </div>
          </div>
        </>
      )}

      <div className="flex items-center gap-2 pt-1">
        <label className={labelCls} style={labelStyle}>启用</label>
        <Toggle
          checked={form.enabled !== false}
          onChange={(v) => setForm((f) => ({ ...f, enabled: v }))}
        />
      </div>

      {!isNew && job.id && (
        <div className="pt-1">
          <button
            onClick={() => setShowLogs(!showLogs)}
            className="flex w-full cursor-pointer items-center gap-1 text-[11px] font-medium transition-colors"
            style={{ color: "var(--fill-tertiary)" }}
          >
            {showLogs ? <ChevronDown size={10} strokeWidth={2} /> : <ChevronRight size={10} strokeWidth={2} />}
            执行记录 {(job as CronJob).run_count > 0 && `(${(job as CronJob).run_count})`}
          </button>
          {showLogs && <div className="mt-2"><RunLogList jobId={job.id!} /></div>}
        </div>
      )}

      <div className="flex items-center justify-between pt-1">
        <div>
          {!isNew && onDelete && (
            <button
              onClick={onDelete}
              disabled={saving}
              className="rounded-[6px] px-3 py-1.5 text-[12px] font-medium transition-colors hover:opacity-80"
              style={{ color: "var(--red, #e53e3e)" }}
            >
              删除任务
            </button>
          )}
        </div>
        <div className="flex items-center gap-2">
          <button
            onClick={onCancel}
            disabled={saving}
            className="cursor-pointer rounded-[6px] px-3 py-1.5 text-[12px] font-medium transition-colors"
            style={{ color: "var(--fill-secondary)" }}
          >
            取消
          </button>
          <button
            onClick={handleSubmit}
            disabled={saving || !form.name || !form.schedule}
            className="cursor-pointer rounded-[6px] px-4 py-1.5 text-[12px] font-medium transition-colors hover:opacity-90 disabled:opacity-50"
            style={{ background: "var(--fill-primary)", color: "var(--fill-inverse)" }}
          >
            {saving ? "保存中..." : "保存"}
          </button>
        </div>
      </div>
    </div>
  );
}

function CronTab() {
  const activeAgentId = useAgentStore((s) => s.activeAgentId);
  const gatewayReady = useGatewayStore((s) => s.connected);

  const [jobs, setJobs] = useState<CronJob[]>([]);
  const [loading, setLoading] = useState(false);
  const [adding, setAdding] = useState(false);
  const [editing, setEditing] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  const loadJobs = useCallback(async () => {
    if (!gatewayReady) return;
    setLoading(true);
    const list = await api.listCronJobs(activeAgentId);
    setJobs(list);
    setLoading(false);
  }, [activeAgentId, gatewayReady]);

  useEffect(() => {
    loadJobs();
  }, [loadJobs]);

  const handleCreate = useCallback(async (job: Partial<CronJob> & { schedule: string; action: CronJobAction }) => {
    setSaving(true);
    try {
      await api.upsertCronJob({
        id: "",
        name: job.name ?? "Unnamed",
        schedule: job.schedule,
        action: job.action,
        enabled: job.enabled !== false,
        status: "idle",
        run_count: 0,
        error_count: 0,
        created_at: new Date().toISOString(),
        last_run: null,
        next_run: null,
        last_error: null,
      } as CronJob & { schedule: string; action: CronJobAction });
      setAdding(false);
      await loadJobs();
    } catch (e) {
      console.error("[cron] create failed:", e);
    }
    setSaving(false);
  }, [loadJobs]);

  const handleUpdate = useCallback(async (job: Partial<CronJob> & { schedule: string; action: CronJobAction }) => {
    if (!editing) return;
    setSaving(true);
    try {
      const existing = jobs.find((j) => j.id === editing);
      if (existing) {
        await api.upsertCronJob({
          ...existing,
          ...job,
        } as CronJob & { schedule: string; action: CronJobAction });
      }
      setEditing(null);
      await loadJobs();
    } catch (e) {
      console.error("[cron] update failed:", e);
    }
    setSaving(false);
  }, [editing, jobs, loadJobs]);

  const handleDelete = useCallback(async (jobId: string) => {
    setSaving(true);
    try {
      await api.deleteCronJob(jobId);
      setEditing(null);
      await loadJobs();
    } catch (e) {
      console.error("[cron] delete failed:", e);
    }
    setSaving(false);
  }, [loadJobs]);

  const handleToggle = useCallback(async (jobId: string, enabled: boolean) => {
    const job = jobs.find((j) => j.id === jobId);
    if (!job) return;
    setJobs((prev) => prev.map((j) => j.id === jobId ? { ...j, enabled } : j));
    try {
      await api.upsertCronJob({ ...job, enabled } as CronJob & { schedule: string; action: CronJobAction });
    } catch {
      setJobs((prev) => prev.map((j) => j.id === jobId ? { ...j, enabled: !enabled } : j));
    }
  }, [jobs]);

  const editingJob = editing ? jobs.find((j) => j.id === editing) : null;

  const formatStatus = (job: CronJob) => {
    if (job.status === "running") return "运行中";
    if (job.status === "failed") return "失败";
    if (!job.enabled) return "已禁用";
    return "空闲";
  };

  const statusColor = (job: CronJob) => {
    if (job.status === "running") return "var(--blue, #4299e1)";
    if (job.status === "failed") return "var(--red, #e53e3e)";
    if (!job.enabled) return "var(--fill-quaternary)";
    return "var(--green, #48bb78)";
  };

  return (
    <div className="space-y-4 p-4">
      <div className="flex items-center justify-between">
        <SectionHeader count={jobs.filter((j) => j.enabled).length} total={jobs.length}>
          定时任务
        </SectionHeader>
        <div className="flex items-center gap-1">
          <button
            onClick={loadJobs}
            disabled={loading}
            className="cursor-pointer rounded-[var(--radius-xs)] p-1.5 transition-colors duration-100 hover:bg-[var(--bg-hover)] disabled:opacity-40"
            title="刷新"
          >
            <RefreshCw size={13} strokeWidth={1.5} className={loading ? "animate-spin" : ""} style={{ color: "var(--fill-tertiary)" }} />
          </button>
          {!adding && (
            <button
              onClick={() => { setAdding(true); setEditing(null); }}
              className="flex cursor-pointer items-center gap-1 rounded-[var(--radius-xs)] p-1.5 text-[11px] font-medium transition-colors hover:bg-[var(--bg-hover)]"
              style={{ color: "var(--fill-tertiary)" }}
            >
              <Plus size={11} strokeWidth={2} /> 新增
            </button>
          )}
        </div>
      </div>

      <FormModal open={adding} onClose={() => setAdding(false)} title="新增定时任务">
        <CronJobForm
          job={{ ...EMPTY_CRON_JOB, action: { ...EMPTY_CRON_JOB.action, agent_id: activeAgentId } }}
          agentId={activeAgentId}
          isNew
          onSave={handleCreate}
          onCancel={() => setAdding(false)}
          saving={saving}
        />
      </FormModal>

      {editingJob && (
        <FormModal open onClose={() => setEditing(null)} title={`编辑 — ${editingJob.name}`}>
          <CronJobForm
            job={editingJob}
            agentId={activeAgentId}
            isNew={false}
            onSave={handleUpdate}
            onCancel={() => setEditing(null)}
            onDelete={() => handleDelete(editingJob.id)}
            saving={saving}
          />
        </FormModal>
      )}

      {jobs.length === 0 ? (
        <ListContainer>
          <div className="px-3 py-6 text-center">
            <Clock size={18} strokeWidth={1.5} className="mx-auto mb-2" style={{ color: "var(--fill-quaternary)" }} />
            <p className="text-[12px]" style={{ color: "var(--fill-tertiary)" }}>
              暂无定时任务
            </p>
            <p className="mt-1 text-[11px]" style={{ color: "var(--fill-quaternary)" }}>
              添加定时任务，Agent 将按设定时间自动执行
            </p>
          </div>
        </ListContainer>
      ) : (
        <ListContainer>
          {jobs.map((job, i) => (
            <div
              key={job.id}
              className="group cursor-pointer px-3 py-2.5 transition-colors duration-100 hover:bg-[var(--bg-hover)]"
              style={{ borderBottom: i < jobs.length - 1 ? "0.5px solid var(--separator)" : undefined }}
              onClick={() => { setEditing(job.id); setAdding(false); }}
            >
              <div className="flex items-center justify-between gap-2">
                <div className="flex min-w-0 flex-1 items-center gap-2">
                  <Clock size={13} strokeWidth={1.5} style={{ color: "var(--fill-tertiary)" }} />
                  <span className="truncate text-[13px] font-medium" style={{ color: "var(--fill-primary)" }}>
                    {job.name}
                  </span>
                  <span
                    className="inline-block h-[6px] w-[6px] shrink-0 rounded-full"
                    style={{ background: statusColor(job) }}
                    title={formatStatus(job)}
                  />
                </div>
                <div className="flex items-center gap-2" onClick={(e) => e.stopPropagation()}>
                  <Toggle
                    checked={job.enabled}
                    onChange={(v) => { handleToggle(job.id, v); }}
                  />
                  <Pencil size={12} strokeWidth={1.5} className="shrink-0 opacity-0 transition-opacity group-hover:opacity-100" style={{ color: "var(--fill-quaternary)" }} />
                </div>
              </div>
              <div className="mt-1 flex items-center gap-3 text-[11px]" style={{ color: "var(--fill-quaternary)" }}>
                <span className="font-mono">{job.schedule}</span>
                <span>·</span>
                <span>{job.action.type === "agent_chat" ? "Agent 对话" : "Webhook"}</span>
                {job.run_count > 0 && (
                  <>
                    <span>·</span>
                    <span>已执行 {job.run_count} 次</span>
                  </>
                )}
              </div>
              {job.last_error && (
                <div className="mt-1 flex items-center gap-1 text-[10px]" style={{ color: "var(--red, #e53e3e)" }}>
                  <AlertTriangle size={10} strokeWidth={1.5} />
                  <span className="truncate">{job.last_error}</span>
                </div>
              )}
              {job.next_run && (
                <div className="mt-0.5 text-[10px]" style={{ color: "var(--fill-quaternary)" }}>
                  下次执行: {new Date(job.next_run).toLocaleString("zh-CN")}
                </div>
              )}
            </div>
          ))}
        </ListContainer>
      )}
    </div>
  );
}

/* ━━━ Main Panel ━━━ */

export function AgentDetail({ open, onClose, agentName, agentInitial }: AgentDetailProps) {
  const [tab, setTab] = useState<Tab>("config");
  const activeAgentId = useAgentStore((s) => s.activeAgentId);
  const agents = useAgentStore((s) => s.agents);
  const agent = agents.find((a) => a.id === activeAgentId);
  const [avatarPreview, setAvatarPreview] = useState<string | null>(null);

  useEffect(() => {
    setAvatarPreview(null);
  }, [activeAgentId]);

  const handleAvatarClick = useCallback(async () => {
    if (!transport.isTauri) return;
    try {
      const { open: openDialog } = await import("@tauri-apps/plugin-dialog");
      const selected = await openDialog({ filters: [{ name: "Images", extensions: ["png", "jpg", "jpeg", "webp"] }], multiple: false });
      if (selected) {
        const path = typeof selected === "string" ? selected : (selected as { path?: string }).path;
        if (path) {
          const resp = await api.uploadAgentAvatar(activeAgentId, path);
          if (resp) {
            const { convertFileSrc } = await import("@tauri-apps/api/core");
            setAvatarPreview(convertFileSrc(resp));
          }
        }
      }
    } catch { /* silent */ }
  }, [activeAgentId]);

  const avatarSrc = avatarPreview || (agent?.avatar ? (() => { try { return new URL(agent.avatar!).href; } catch { return undefined; } })() : undefined);

  return (
    <aside
      className="flex shrink-0 flex-col overflow-hidden transition-all duration-300 ease-out"
      style={{
        width: open ? 320 : 0,
        opacity: open ? 1 : 0,
        borderLeft: open ? "0.5px solid var(--separator)" : "none",
        background: "var(--bg-secondary)",
      }}
    >
      <div className="flex shrink-0 items-center justify-between gap-2 px-4 py-3.5" style={{ borderBottom: "0.5px solid var(--separator)" }}>
        <div className="flex min-w-0 flex-1 items-center gap-2.5">
          <button
            className="group relative flex h-8 w-8 shrink-0 cursor-pointer items-center justify-center overflow-hidden rounded-full text-[12px] font-semibold"
            style={{ background: "var(--bg-tertiary)", color: "var(--fill-secondary)" }}
            onClick={handleAvatarClick}
            title="修改头像"
          >
            {avatarSrc ? (
              <img src={avatarSrc} alt="" className="h-full w-full object-cover" />
            ) : (
              agentInitial
            )}
            <div className="absolute inset-0 flex items-center justify-center rounded-full opacity-0 transition-opacity duration-100 group-hover:opacity-100" style={{ background: "rgba(0,0,0,0.3)" }}>
              <Camera size={12} strokeWidth={1.5} color="white" />
            </div>
          </button>
          <span className="min-w-0 truncate text-[14px] font-semibold" style={{ color: "var(--fill-primary)" }} title={agentName}>{agentName}</span>
        </div>
        <button onClick={onClose} className="flex h-7 w-7 shrink-0 cursor-pointer items-center justify-center rounded-full transition-colors duration-100 hover:bg-[var(--bg-hover)]" style={{ color: "var(--fill-tertiary)" }} title="关闭面板">
          <X size={14} strokeWidth={1.5} />
        </button>
      </div>

      <div className="flex shrink-0 px-4 pt-3 pb-1">
        <div className="flex w-full rounded-[var(--radius-xs)] p-0.5" style={{ background: "var(--bg-tertiary)" }}>
          {(["config", "chats", "cron"] as const).map((t) => (
            <button
              key={t}
              onClick={() => setTab(t)}
              className="flex-1 cursor-pointer rounded-[4px] py-1.5 text-center text-[12px] font-medium transition-all duration-200"
              style={{
                background: tab === t ? "var(--bg-elevated)" : "transparent",
                color: tab === t ? "var(--fill-primary)" : "var(--fill-tertiary)",
                boxShadow: tab === t ? "var(--shadow-sm)" : "none",
              }}
            >
              {t === "config" ? "配置" : t === "chats" ? "会话" : "定时"}
            </button>
          ))}
        </div>
      </div>

      <div className="flex-1 overflow-y-auto">
        {tab === "config" ? <ConfigTab key={activeAgentId} /> : tab === "chats" ? <ChatsTab /> : <CronTab key={`cron-${activeAgentId}`} />}
      </div>
    </aside>
  );
}
