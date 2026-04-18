import { useState, useEffect, useCallback } from "react";
import { useThemeStore, type ThemeMode } from "../../lib/theme";
import { useGatewayStore } from "../../lib/store";
import { Settings2, Box, Wrench, Server, Info, ChevronDown, Plus, Pencil, Globe, User, X, RefreshCw, Upload, FolderOpen, FileText, Plug, Trash2, ToggleLeft, ToggleRight } from "lucide-react";
import { ClawIcon } from "../layout/ClawIcon";
import * as api from "../../lib/api";

interface SettingsPanelProps {
  open: boolean;
  onClose: () => void;
}

type SettingsTab = "general" | "models" | "skills" | "mcp" | "gateway" | "about";

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
      className="flex h-[22px] w-[40px] shrink-0 items-center rounded-full px-0.5 transition-colors duration-200"
      style={{
        background: enabled ? "var(--green)" : "var(--fill-quaternary)",
        justifyContent: enabled ? "flex-end" : "flex-start",
      }}
    >
      <div className="h-[18px] w-[18px] rounded-full bg-white shadow-sm" />
    </button>
  );
}

/* ━━━ General Tab ━━━ */

function GeneralTab() {
  const { mode, setMode } = useThemeStore();
  const [notifications, setNotifications] = useState(true);
  const [sounds, setSounds] = useState(false);
  const [autoScroll, setAutoScroll] = useState(true);
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
  }, []);

  const persist = useCallback((key: string, value: boolean) => {
    api.setConfig("session", { [key]: value }).catch(() => {});
  }, []);

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
              className="flex-1 rounded-[4px] py-1.5 text-center text-[12px] font-medium transition-all duration-200"
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
};

function ModelForm({
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
  const patch = (k: string, v: string | number) => setForm((f) => ({ ...f, [k]: v }));

  const inputStyle = { background: "var(--bg-base)", color: "var(--fill-primary)", border: "0.5px solid var(--separator-opaque)" };

  return (
    <div className="space-y-3 rounded-[var(--radius-xs)] p-4" style={{ background: "var(--bg-elevated)", border: "0.5px solid var(--separator-opaque)", boxShadow: "var(--shadow-sm)" }}>
      <div className="grid grid-cols-2 gap-3">
        <div>
          <label className="mb-1 block text-[11px] font-medium" style={{ color: "var(--fill-tertiary)" }}>名称 (key)</label>
          <input
            value={form.key}
            onChange={(e) => patch("key", e.target.value)}
            disabled={!isNew}
            placeholder="例: dashscope"
            className="w-full rounded-[6px] px-3 py-2 text-[13px] outline-none transition-colors focus:ring-2 focus:ring-[var(--tint)]"
            style={{ ...inputStyle, opacity: isNew ? 1 : 0.7 }}
          />
        </div>
        <div>
          <label className="mb-1 block text-[11px] font-medium" style={{ color: "var(--fill-tertiary)" }}>Provider</label>
          <div className="relative">
            <select
              value={form.provider}
              onChange={(e) => patch("provider", e.target.value)}
              className="w-full cursor-pointer rounded-[6px] px-3 py-2 pr-8 text-[13px] outline-none transition-colors focus:ring-2 focus:ring-[var(--tint)]"
              style={{ ...inputStyle, appearance: "none" }}
            >
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
          <label className="mb-1 block text-[11px] font-medium" style={{ color: "var(--fill-tertiary)" }}>模型名称</label>
          <input
            value={form.model}
            onChange={(e) => patch("model", e.target.value)}
            placeholder="例: gpt-4o"
            className="w-full rounded-[6px] px-3 py-2 text-[13px] outline-none transition-colors focus:ring-2 focus:ring-[var(--tint)]"
            style={inputStyle}
          />
        </div>
        <div>
          <label className="mb-1 block text-[11px] font-medium" style={{ color: "var(--fill-tertiary)" }}>Base URL</label>
          <input
            value={form.baseUrl}
            onChange={(e) => patch("baseUrl", e.target.value)}
            placeholder="https://api.openai.com/v1"
            className="w-full rounded-[6px] px-3 py-2 text-[13px] outline-none transition-colors focus:ring-2 focus:ring-[var(--tint)]"
            style={inputStyle}
          />
        </div>
      </div>
      <div>
        <label className="mb-1 block text-[11px] font-medium" style={{ color: "var(--fill-tertiary)" }}>API Key</label>
        <input
          type="password"
          value={cred.apiKey}
          onChange={(e) => setCred((c) => ({ ...c, apiKey: e.target.value }))}
          placeholder="sk-..."
          className="w-full rounded-[6px] px-3 py-2 text-[13px] font-mono outline-none transition-colors focus:ring-2 focus:ring-[var(--tint)]"
          style={inputStyle}
        />
      </div>
      <div className="grid grid-cols-3 gap-3">
        <div>
          <label className="mb-1 block text-[11px] font-medium" style={{ color: "var(--fill-tertiary)" }}>温度</label>
          <input
            type="number"
            step="0.1"
            min="0"
            max="2"
            value={form.temperature}
            onChange={(e) => patch("temperature", parseFloat(e.target.value) || 0)}
            className="w-full rounded-[6px] px-3 py-2 text-[13px] outline-none transition-colors focus:ring-2 focus:ring-[var(--tint)]"
            style={inputStyle}
          />
        </div>
        <div>
          <label className="mb-1 block text-[11px] font-medium" style={{ color: "var(--fill-tertiary)" }}>并发数</label>
          <input
            type="number"
            min="1"
            value={form.maxConcurrent}
            onChange={(e) => patch("maxConcurrent", parseInt(e.target.value) || 1)}
            className="w-full rounded-[6px] px-3 py-2 text-[13px] outline-none transition-colors focus:ring-2 focus:ring-[var(--tint)]"
            style={inputStyle}
          />
        </div>
        <div>
          <label className="mb-1 block text-[11px] font-medium" style={{ color: "var(--fill-tertiary)" }}>超时 (秒)</label>
          <input
            type="number"
            min="10"
            value={form.timeoutSecs}
            onChange={(e) => patch("timeoutSecs", parseInt(e.target.value) || 60)}
            className="w-full rounded-[6px] px-3 py-2 text-[13px] outline-none transition-colors focus:ring-2 focus:ring-[var(--tint)]"
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
              style={{ color: "var(--red)" }}
            >
              删除
            </button>
          )}
        </div>
        <div className="flex items-center gap-2">
          <button
            onClick={onCancel}
            disabled={saving}
            className="rounded-[6px] px-3 py-1.5 text-[12px] font-medium transition-colors"
            style={{ color: "var(--fill-secondary)" }}
          >
            取消
          </button>
          <button
            onClick={() => onSave(form, cred)}
            disabled={saving || !form.key || !form.model}
            className="rounded-[6px] px-4 py-1.5 text-[12px] font-medium text-white transition-colors hover:opacity-90 disabled:opacity-50"
            style={{ background: "var(--tint)" }}
          >
            {saving ? "保存中..." : "保存"}
          </button>
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
    }));

  const handleSave = async (entry: ModelConfigEntry, cred: CredentialEntry) => {
    setSaving(true);
    try {
      const newModels = { ...modelsConfig };
      if (editing && editing !== entry.key) {
        delete newModels[editing];
      }
      newModels[entry.key] = {
        provider: entry.provider,
        model: entry.model,
        baseUrl: entry.baseUrl,
        temperature: entry.temperature,
        maxConcurrent: entry.maxConcurrent,
        timeoutSecs: entry.timeoutSecs,
      };
      await api.setConfig("models", newModels);

      if (cred.apiKey && !cred.apiKey.startsWith("***")) {
        const newCreds = { ...credentials };
        newCreds[entry.key] = cred;
        await api.setConfig("credentials", newCreds);
      }

      setEditing(null);
      setAdding(false);
      loadData();
    } catch {
      /* silent */
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
    } catch {
      /* silent */
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
      <div className="flex items-center justify-between">
        <SectionTitle>已配置模型 ({entries.length})</SectionTitle>
        {!adding && (
          <button
            onClick={() => { setAdding(true); setEditing(null); }}
            className="flex items-center gap-1 rounded-[6px] px-2.5 py-1 text-[12px] font-medium transition-colors hover:opacity-80"
            style={{ color: "var(--tint)" }}
          >
            <Plus size={12} strokeWidth={2} />
            新增模型
          </button>
        )}
      </div>

      {adding && (
        <ModelForm
          entry={{ key: "", ...EMPTY_MODEL }}
          credential={{ apiKey: "", baseUrl: "" }}
          isNew
          onSave={handleSave}
          onCancel={() => setAdding(false)}
          saving={saving}
        />
      )}

      <div className="overflow-hidden rounded-[var(--radius-sm)]" style={{ background: "var(--bg-elevated)", border: "0.5px solid var(--separator-opaque)" }}>
        {entries.map((entry, idx) =>
          editing === entry.key ? (
            <ModelForm
              key={entry.key}
              entry={entry}
              credential={credentials[entry.key] ?? { apiKey: "", baseUrl: "" }}
              isNew={false}
              onSave={handleSave}
              onCancel={() => setEditing(null)}
              onDelete={() => handleDelete(entry.key)}
              saving={saving}
            />
          ) : (
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
          )
        )}
      </div>

      {entries.length === 0 && !adding && (
        <div className="py-8 text-center">
          <p className="text-[13px]" style={{ color: "var(--fill-tertiary)" }}>
            暂无已配置模型，点击上方"新增模型"添加
          </p>
        </div>
      )}

      <p className="text-[11px]" style={{ color: "var(--fill-quaternary)" }}>
        点击模型卡片即可编辑，修改会持久化到 ~/.fastclaw/config/default.json（部分配置重启后生效）
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

  if (loading) {
    return (
      <div className="flex items-center justify-center py-12">
        <span className="text-[13px]" style={{ color: "var(--fill-tertiary)" }}>加载中...</span>
      </div>
    );
  }

  const totalSkills = publicSkills.length + Object.values(agentSkillsMap).reduce((s, a) => s + a.length, 0);

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
                  onClick={() => {
                    const el = document.getElementById("settings-skill-upload-menu");
                    if (el) el.classList.toggle("hidden");
                  }}
                  disabled={uploading}
                  className="cursor-pointer rounded-[var(--radius-xs)] p-1.5 transition-colors duration-100 hover:bg-[var(--bg-hover)] disabled:opacity-40"
                  title="上传 Skill"
                >
                  <Upload size={13} strokeWidth={1.5} style={{ color: "var(--fill-tertiary)" }} />
                </button>
                <div
                  id="settings-skill-upload-menu"
                  className="hidden absolute right-0 top-full z-50 mt-1 min-w-[140px] overflow-hidden rounded-[var(--radius-sm)] py-1 shadow-lg"
                  style={{ background: "var(--bg-elevated)", border: "0.5px solid var(--separator-opaque)" }}
                  onMouseLeave={(e) => (e.currentTarget as HTMLElement).classList.add("hidden")}
                >
                  <button
                    onClick={() => { document.getElementById("settings-skill-upload-menu")?.classList.add("hidden"); handleUploadFolder(); }}
                    className="w-full cursor-pointer px-3 py-2 text-left text-[12px] transition-colors hover:bg-[var(--bg-hover)]"
                    style={{ color: "var(--fill-primary)" }}
                  >
                    <FolderOpen size={12} className="mr-2 inline" strokeWidth={1.5} />选择文件夹
                  </button>
                  <button
                    onClick={() => { document.getElementById("settings-skill-upload-menu")?.classList.add("hidden"); handleUploadZip(); }}
                    className="w-full cursor-pointer px-3 py-2 text-left text-[12px] transition-colors hover:bg-[var(--bg-hover)]"
                    style={{ color: "var(--fill-primary)" }}
                  >
                    <FileText size={12} className="mr-2 inline" strokeWidth={1.5} />选择 ZIP 文件
                  </button>
                </div>
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
        const agentId = "main";
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

function McpTab() {
  const [servers, setServers] = useState<McpServerEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const [adding, setAdding] = useState(false);
  const [editingId, setEditingId] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
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

  useEffect(() => { loadServers(); }, [loadServers]);

  const persist = async (updated: McpServerEntry[]) => {
    setSaving(true);
    try {
      await api.setConfig("mcpServers", updated);
      setServers(updated);
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
        {!adding && !editingId && (
          <button onClick={startAdd} className="flex cursor-pointer items-center gap-1.5 rounded-[var(--radius-xs)] px-2.5 py-1 text-[12px] font-medium transition-colors duration-100 hover:bg-[var(--bg-hover)]" style={{ color: "var(--accent)" }}>
            <Plus size={13} strokeWidth={2} /> 添加
          </button>
        )}
      </div>

      {adding && renderForm()}

      {servers.length === 0 && !adding && (
        <div className="rounded-[var(--radius-sm)] py-8 text-center text-[13px]" style={{ color: "var(--fill-tertiary)", background: "var(--bg-elevated)", border: "0.5px solid var(--separator-opaque)" }}>
          暂无全局 MCP 服务器
        </div>
      )}

      {servers.map((srv) => (
        editingId === srv.id ? (
          <div key={srv.id}>{renderForm()}</div>
        ) : (
          <div key={srv.id} className="flex items-center gap-3 rounded-[var(--radius-sm)] px-4 py-3" style={{ background: "var(--bg-elevated)", border: "0.5px solid var(--separator-opaque)", opacity: srv.enabled ? 1 : 0.55 }}>
            <Plug size={16} strokeWidth={1.5} style={{ color: srv.enabled ? "var(--accent)" : "var(--fill-quaternary)", flexShrink: 0 }} />
            <div className="min-w-0 flex-1">
              <div className="text-[13px] font-semibold font-mono" style={{ color: "var(--fill-primary)" }}>{srv.id}</div>
              <div className="mt-0.5 truncate text-[11px] font-mono" style={{ color: "var(--fill-tertiary)" }} title={[srv.command, ...srv.args].join(" ")}>{srv.command} {srv.args.join(" ")}</div>
            </div>
            <div className="flex items-center gap-1">
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
        )
      ))}

      {saving && <div className="text-center text-[11px]" style={{ color: "var(--fill-tertiary)" }}>保存中…</div>}

      <p className="text-[11px]" style={{ color: "var(--fill-quaternary)" }}>
        MCP 服务器配置变更需重启网关后生效
      </p>

      <McpToolsList />
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
  { id: "skills", label: "Skills", icon: <Wrench {...ICON_PROPS} /> },
  { id: "mcp", label: "MCP", icon: <Plug {...ICON_PROPS} /> },
  { id: "gateway", label: "网关", icon: <Server {...ICON_PROPS} /> },
  { id: "about", label: "关于", icon: <Info {...ICON_PROPS} /> },
];

export function SettingsPanel({ open, onClose }: SettingsPanelProps) {
  const [tab, setTab] = useState<SettingsTab>("general");

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
            {tab === "skills" && <SkillsTab />}
            {tab === "mcp" && <McpTab />}
            {tab === "gateway" && <GatewayTab />}
            {tab === "about" && <AboutTab />}
          </div>
        </div>
      </div>
    </div>
  );
}
