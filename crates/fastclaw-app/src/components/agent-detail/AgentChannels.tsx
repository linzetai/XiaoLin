import { useState, useEffect, useCallback } from "react";
import { ChevronDown, Link2, Plus, Pencil } from "lucide-react";
import * as api from "../../lib/api";
import { FormModal, ListContainer, Toggle } from "./common";

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

export function ChannelManager({ agentId, backendAgent, ready }: {
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
