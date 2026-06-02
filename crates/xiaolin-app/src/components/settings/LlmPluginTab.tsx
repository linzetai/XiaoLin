import { useState, useEffect, useCallback } from "react";
import {
  Plus, Trash2, RefreshCw, CheckCircle2, XCircle, Pencil,
  Zap, Terminal, ChevronDown, ChevronUp, Play,
} from "lucide-react";
import { SectionTitle } from "./SettingsShared";
import {
  listLlmPlugins, createLlmPlugin, updateLlmPlugin, deleteLlmPlugin,
  testLlmPlugin, type LlmPluginSummary,
} from "../../lib/api";
import { ICON } from "../../lib/ui-tokens";

type AuthType = "none" | "bearer_token" | "custom_header" | "oauth2_client_credentials" | "pre_request_hook";

interface PluginFormState {
  id: string;
  name: string;
  version: string;
  description: string;
  type: "middleware" | "process";
  enabled: boolean;
  // Middleware
  baseUrl: string;
  protocol: "openai" | "anthropic";
  headers: Record<string, string>;
  authType: AuthType;
  authToken: string;
  authHeader: string;
  authValue: string;
  oauth2TokenEndpoint: string;
  oauth2ClientId: string;
  oauth2ClientSecret: string;
  oauth2Scope: string;
  preRequestUrl: string;
  preRequestMethod: string;
  preRequestExtractPath: string;
  preRequestCacheTtl: number;
  modelMapping: Record<string, string>;
  // Process
  command: string;
  args: string;
  env: Record<string, string>;
  // Models
  models: { id: string; name: string; contextWindow: number }[];
}

const emptyForm = (): PluginFormState => ({
  id: "", name: "", version: "1.0.0", description: "",
  type: "middleware", enabled: true,
  baseUrl: "", protocol: "openai",
  headers: {}, authType: "none",
  authToken: "", authHeader: "", authValue: "",
  oauth2TokenEndpoint: "", oauth2ClientId: "", oauth2ClientSecret: "", oauth2Scope: "",
  preRequestUrl: "", preRequestMethod: "POST", preRequestExtractPath: "access_token", preRequestCacheTtl: 300,
  modelMapping: {},
  command: "", args: "", env: {},
  models: [],
});

function formToPayload(f: PluginFormState): Record<string, unknown> {
  const payload: Record<string, unknown> = {
    id: f.id, name: f.name, version: f.version, description: f.description,
    type: f.type, enabled: f.enabled, models: f.models,
  };
  if (f.type === "middleware") {
    const auth: Record<string, unknown> = { type: f.authType };
    switch (f.authType) {
      case "bearer_token": auth.token = f.authToken; break;
      case "custom_header": auth.header = f.authHeader; auth.value = f.authValue; break;
      case "oauth2_client_credentials":
        auth.tokenEndpoint = f.oauth2TokenEndpoint;
        auth.clientId = f.oauth2ClientId;
        auth.clientSecret = f.oauth2ClientSecret;
        if (f.oauth2Scope) auth.scope = f.oauth2Scope;
        break;
      case "pre_request_hook":
        auth.url = f.preRequestUrl;
        auth.method = f.preRequestMethod;
        auth.extractPath = f.preRequestExtractPath;
        auth.cacheTtlSecs = f.preRequestCacheTtl;
        break;
    }
    payload.middleware = {
      baseUrl: f.baseUrl, protocol: f.protocol,
      headers: f.headers, auth,
      modelMapping: f.modelMapping,
    };
  } else {
    payload.process = {
      command: f.command,
      args: f.args ? f.args.split(/\s+/).filter(Boolean) : [],
      env: f.env, transport: "stdio",
    };
  }
  return payload;
}

const BTN_BASE = "flex cursor-pointer items-center gap-1.5 rounded-[var(--radius-xs)] px-3 py-1.5 text-[12px] font-medium transition-all duration-150";

function Btn({ children, onClick, variant = "default", disabled = false }: {
  children: React.ReactNode; onClick?: () => void; variant?: "default" | "primary" | "danger"; disabled?: boolean;
}) {
  const styles: Record<string, React.CSSProperties> = {
    default: { background: "var(--bg-secondary)", color: "var(--fill-secondary)", border: "0.5px solid var(--separator)" },
    primary: { background: "var(--tint)", color: "#fff" },
    danger: { background: "var(--bg-secondary)", color: "var(--system-red)", border: "0.5px solid var(--separator)" },
  };
  return (
    <button className={BTN_BASE} style={{ ...styles[variant], opacity: disabled ? 0.5 : 1 }} onClick={onClick} disabled={disabled}>
      {children}
    </button>
  );
}

function Input({ value, onChange, placeholder, type = "text", className = "" }: {
  value: string; onChange: (v: string) => void; placeholder?: string; type?: string; className?: string;
}) {
  return (
    <input
      type={type}
      value={value}
      onChange={(e) => onChange(e.target.value)}
      placeholder={placeholder}
      className={`w-full rounded-[var(--radius-xs)] px-3 py-2 text-[13px] outline-none ${className}`}
      style={{ background: "var(--bg-primary)", border: "0.5px solid var(--separator)", color: "var(--fill-primary)" }}
    />
  );
}

function Select({ value, onChange, options }: {
  value: string; onChange: (v: string) => void; options: { value: string; label: string }[];
}) {
  return (
    <select
      value={value}
      onChange={(e) => onChange(e.target.value)}
      className="select-premium"
    >
      {options.map((o) => <option key={o.value} value={o.value}>{o.label}</option>)}
    </select>
  );
}

function Label({ children }: { children: React.ReactNode }) {
  return <label className="mb-1 block text-[12px] font-medium" style={{ color: "var(--fill-secondary)" }}>{children}</label>;
}

export function LlmPluginTab() {
  const [plugins, setPlugins] = useState<LlmPluginSummary[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [editing, setEditing] = useState<PluginFormState | null>(null);
  const [isNew, setIsNew] = useState(false);
  const [testResult, setTestResult] = useState<Record<string, { ok: boolean; message: string }>>({});
  const [expanded, setExpanded] = useState<Set<string>>(new Set());

  const refresh = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      setPlugins(await listLlmPlugins());
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => { refresh(); }, [refresh]);

  const handleDelete = async (id: string) => {
    if (!confirm(`确定删除插件 "${id}" 吗?`)) return;
    try {
      await deleteLlmPlugin(id);
      await refresh();
    } catch (e) {
      setError((e as Error).message);
    }
  };

  const handleTest = async (id: string) => {
    setTestResult((prev) => ({ ...prev, [id]: { ok: true, message: "测试中..." } }));
    try {
      const result = await testLlmPlugin(id);
      setTestResult((prev) => ({
        ...prev,
        [id]: result.ok
          ? { ok: true, message: `成功 — 模型: ${result.model}, 回复: ${result.reply}` }
          : { ok: false, message: result.error ?? "未知错误" },
      }));
    } catch (e) {
      setTestResult((prev) => ({ ...prev, [id]: { ok: false, message: (e as Error).message } }));
    }
  };

  const handleSave = async () => {
    if (!editing) return;
    try {
      const payload = formToPayload(editing);
      if (isNew) {
        await createLlmPlugin(payload);
      } else {
        await updateLlmPlugin(editing.id, payload);
      }
      setEditing(null);
      await refresh();
    } catch (e) {
      setError((e as Error).message);
    }
  };

  const openNew = () => {
    setEditing(emptyForm());
    setIsNew(true);
  };

  const openEdit = (p: LlmPluginSummary) => {
    setIsNew(false);
    const form = emptyForm();
    form.id = p.id;
    form.name = p.name;
    form.type = p.type;
    form.enabled = p.enabled;
    form.models = p.models;
    setEditing(form);
  };

  const toggleExpand = (id: string) => {
    setExpanded((prev) => {
      const next = new Set(prev);
      next.has(id) ? next.delete(id) : next.add(id);
      return next;
    });
  };

  if (editing) {
    return <PluginForm form={editing} setForm={setEditing} onSave={handleSave} onCancel={() => setEditing(null)} isNew={isNew} />;
  }

  return (
    <div className="space-y-5">
      <div className="flex items-center justify-between">
        <SectionTitle>LLM 提供商插件</SectionTitle>
        <div className="flex gap-2">
          <Btn onClick={refresh}><RefreshCw {...ICON.sm} /> 刷新</Btn>
          <Btn variant="primary" onClick={openNew}><Plus {...ICON.sm} /> 添加插件</Btn>
        </div>
      </div>

      {error && (
        <div className="rounded-[var(--radius-xs)] px-4 py-2.5 text-[12px]" style={{ background: "var(--system-red-bg)", color: "var(--system-red)" }}>
          {error}
        </div>
      )}

      {loading ? (
        <div className="py-8 text-center text-[13px]" style={{ color: "var(--fill-tertiary)" }}>加载中...</div>
      ) : plugins.length === 0 ? (
        <div className="rounded-[var(--radius-sm)] px-5 py-8 text-center" style={{ background: "var(--bg-secondary)" }}>
          <div className="text-[13px] font-medium" style={{ color: "var(--fill-secondary)" }}>尚未安装任何 LLM 插件</div>
          <div className="mt-1 text-[12px]" style={{ color: "var(--fill-tertiary)" }}>
            插件可以添加自定义 LLM 提供商，支持自定义鉴权、请求头、模型映射等
          </div>
        </div>
      ) : (
        <div className="space-y-2">
          {plugins.map((p) => {
            const isExpanded = expanded.has(p.id);
            const tr = testResult[p.id];
            return (
              <div key={p.id} className="overflow-hidden rounded-[var(--radius-sm)]" style={{ background: "var(--bg-secondary)", border: "0.5px solid var(--separator)" }}>
                <div className="flex items-center gap-3 px-4 py-3 cursor-pointer" onClick={() => toggleExpand(p.id)}>
                  <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-[var(--radius-xs)]" style={{ background: "var(--bg-hover)" }}>
                    {p.type === "middleware" ? <Zap {...ICON.md} style={{ color: "var(--tint)" }} /> : <Terminal {...ICON.md} style={{ color: "var(--tint)" }} />}
                  </div>
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-2">
                      <span className="text-[13px] font-medium" style={{ color: "var(--fill-primary)" }}>{p.name}</span>
                      <span className="rounded-full px-2 py-0.5 text-[10px]" style={{
                        background: p.enabled ? "var(--system-green-bg)" : "var(--bg-hover)",
                        color: p.enabled ? "var(--system-green)" : "var(--fill-tertiary)",
                      }}>
                        {p.enabled ? "启用" : "禁用"}
                      </span>
                      <span className="rounded-full px-2 py-0.5 text-[10px]" style={{ background: "var(--bg-hover)", color: "var(--fill-tertiary)" }}>
                        {p.type === "middleware" ? "中间件" : "外部进程"}
                      </span>
                    </div>
                    <div className="text-[11px]" style={{ color: "var(--fill-tertiary)" }}>
                      {p.models.length > 0
                        ? `${p.models.length} 个模型: ${p.models.map((m) => m.name || m.id).join(", ")}`
                        : "暂无模型定义"}
                    </div>
                  </div>
                  {isExpanded ? <ChevronUp {...ICON.sm} style={{ color: "var(--fill-tertiary)" }} /> : <ChevronDown {...ICON.sm} style={{ color: "var(--fill-tertiary)" }} />}
                </div>
                {isExpanded && (
                  <div className="px-4 pb-3 pt-0">
                    <div className="flex items-center gap-2 mb-2">
                      <span className="text-[11px]" style={{ color: "var(--fill-tertiary)" }}>ID: {p.id}</span>
                      {p.version && <span className="text-[11px]" style={{ color: "var(--fill-tertiary)" }}>v{p.version}</span>}
                    </div>
                    {p.description && <div className="mb-3 text-[12px]" style={{ color: "var(--fill-secondary)" }}>{p.description}</div>}
                    <div className="flex gap-2">
                      <Btn onClick={() => handleTest(p.id)}><Play {...ICON.sm} /> 测试连接</Btn>
                      <Btn onClick={() => openEdit(p)}><Pencil {...ICON.sm} /> 编辑</Btn>
                      <Btn variant="danger" onClick={() => handleDelete(p.id)}><Trash2 {...ICON.sm} /> 删除</Btn>
                    </div>
                    {tr && (
                      <div className="mt-2 flex items-center gap-2 text-[12px]" style={{ color: tr.ok ? "var(--system-green)" : "var(--system-red)" }}>
                        {tr.ok ? <CheckCircle2 {...ICON.sm} /> : <XCircle {...ICON.sm} />}
                        {tr.message}
                      </div>
                    )}
                  </div>
                )}
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}

function PluginForm({ form, setForm, onSave, onCancel, isNew }: {
  form: PluginFormState;
  setForm: (f: PluginFormState | null) => void;
  onSave: () => void;
  onCancel: () => void;
  isNew: boolean;
}) {
  const update = <K extends keyof PluginFormState>(key: K, value: PluginFormState[K]) => {
    setForm({ ...form, [key]: value });
  };

  const addModel = () => {
    update("models", [...form.models, { id: "", name: "", contextWindow: 128000 }]);
  };

  const removeModel = (idx: number) => {
    update("models", form.models.filter((_, i) => i !== idx));
  };

  const updateModel = (idx: number, key: string, value: string | number) => {
    const models = [...form.models];
    models[idx] = { ...models[idx], [key]: value };
    update("models", models);
  };

  const isValid = form.id.trim() !== "" && form.name.trim() !== ""
    && (form.type === "process" ? form.command.trim() !== "" : form.baseUrl.trim() !== "");

  return (
    <div className="space-y-5">
      <div className="flex items-center justify-between">
        <SectionTitle>{isNew ? "添加 LLM 插件" : `编辑: ${form.name}`}</SectionTitle>
        <div className="flex gap-2">
          <Btn onClick={onCancel}>取消</Btn>
          <Btn variant="primary" onClick={onSave} disabled={!isValid}>保存</Btn>
        </div>
      </div>

      <div className="space-y-4 rounded-[var(--radius-sm)] p-4" style={{ background: "var(--bg-secondary)", border: "0.5px solid var(--separator)" }}>
        <div className="grid grid-cols-2 gap-3">
          <div>
            <Label>插件 ID</Label>
            <Input value={form.id} onChange={(v) => update("id", v)} placeholder="corp-gateway" />
          </div>
          <div>
            <Label>名称</Label>
            <Input value={form.name} onChange={(v) => update("name", v)} placeholder="Corporate Gateway" />
          </div>
        </div>

        <div className="grid grid-cols-2 gap-3">
          <div>
            <Label>类型</Label>
            <Select
              value={form.type}
              onChange={(v) => update("type", v as "middleware" | "process")}
              options={[
                { value: "middleware", label: "中间件 (HTTP 代理)" },
                { value: "process", label: "外部进程 (stdio)" },
              ]}
            />
          </div>
          <div>
            <Label>版本</Label>
            <Input value={form.version} onChange={(v) => update("version", v)} placeholder="1.0.0" />
          </div>
        </div>

        <div>
          <Label>描述</Label>
          <Input value={form.description} onChange={(v) => update("description", v)} placeholder="可选描述" />
        </div>
      </div>

      {form.type === "middleware" ? (
        <div className="space-y-4 rounded-[var(--radius-sm)] p-4" style={{ background: "var(--bg-secondary)", border: "0.5px solid var(--separator)" }}>
          <SectionTitle>中间件配置</SectionTitle>
          <div className="grid grid-cols-2 gap-3">
            <div>
              <Label>Base URL</Label>
              <Input value={form.baseUrl} onChange={(v) => update("baseUrl", v)} placeholder="https://llm-gateway.example.com/v1" />
            </div>
            <div>
              <Label>协议</Label>
              <Select
                value={form.protocol}
                onChange={(v) => update("protocol", v as "openai" | "anthropic")}
                options={[
                  { value: "openai", label: "OpenAI 兼容" },
                  { value: "anthropic", label: "Anthropic" },
                ]}
              />
            </div>
          </div>

          <div>
            <Label>认证方式</Label>
            <Select
              value={form.authType}
              onChange={(v) => update("authType", v as AuthType)}
              options={[
                { value: "none", label: "无认证" },
                { value: "bearer_token", label: "Bearer Token" },
                { value: "custom_header", label: "自定义请求头" },
                { value: "oauth2_client_credentials", label: "OAuth2 Client Credentials" },
                { value: "pre_request_hook", label: "Pre-Request Hook" },
              ]}
            />
          </div>

          {form.authType === "bearer_token" && (
            <div>
              <Label>Token</Label>
              <Input value={form.authToken} onChange={(v) => update("authToken", v)} placeholder="sk-..." type="password" />
            </div>
          )}

          {form.authType === "custom_header" && (
            <div className="grid grid-cols-2 gap-3">
              <div>
                <Label>Header 名称</Label>
                <Input value={form.authHeader} onChange={(v) => update("authHeader", v)} placeholder="x-api-key" />
              </div>
              <div>
                <Label>Header 值</Label>
                <Input value={form.authValue} onChange={(v) => update("authValue", v)} placeholder="secret-key" type="password" />
              </div>
            </div>
          )}

          {form.authType === "oauth2_client_credentials" && (
            <div className="space-y-3">
              <div>
                <Label>Token Endpoint</Label>
                <Input value={form.oauth2TokenEndpoint} onChange={(v) => update("oauth2TokenEndpoint", v)} placeholder="https://auth.example.com/token" />
              </div>
              <div className="grid grid-cols-2 gap-3">
                <div>
                  <Label>Client ID</Label>
                  <Input value={form.oauth2ClientId} onChange={(v) => update("oauth2ClientId", v)} />
                </div>
                <div>
                  <Label>Client Secret</Label>
                  <Input value={form.oauth2ClientSecret} onChange={(v) => update("oauth2ClientSecret", v)} type="password" />
                </div>
              </div>
              <div>
                <Label>Scope (可选)</Label>
                <Input value={form.oauth2Scope} onChange={(v) => update("oauth2Scope", v)} placeholder="llm:invoke" />
              </div>
            </div>
          )}

          {form.authType === "pre_request_hook" && (
            <div className="space-y-3">
              <div className="grid grid-cols-2 gap-3">
                <div>
                  <Label>Hook URL</Label>
                  <Input value={form.preRequestUrl} onChange={(v) => update("preRequestUrl", v)} placeholder="https://auth.internal/token" />
                </div>
                <div>
                  <Label>Method</Label>
                  <Select value={form.preRequestMethod} onChange={(v) => update("preRequestMethod", v)} options={[
                    { value: "POST", label: "POST" },
                    { value: "GET", label: "GET" },
                    { value: "PUT", label: "PUT" },
                  ]} />
                </div>
              </div>
              <div className="grid grid-cols-2 gap-3">
                <div>
                  <Label>提取路径 (dot-separated)</Label>
                  <Input value={form.preRequestExtractPath} onChange={(v) => update("preRequestExtractPath", v)} placeholder="data.access_token" />
                </div>
                <div>
                  <Label>缓存 TTL (秒)</Label>
                  <Input value={String(form.preRequestCacheTtl)} onChange={(v) => update("preRequestCacheTtl", parseInt(v) || 0)} type="number" />
                </div>
              </div>
            </div>
          )}
        </div>
      ) : (
        <div className="space-y-4 rounded-[var(--radius-sm)] p-4" style={{ background: "var(--bg-secondary)", border: "0.5px solid var(--separator)" }}>
          <SectionTitle>进程配置</SectionTitle>
          <div>
            <Label>命令</Label>
            <Input value={form.command} onChange={(v) => update("command", v)} placeholder="python3" />
          </div>
          <div>
            <Label>参数 (空格分隔)</Label>
            <Input value={form.args} onChange={(v) => update("args", v)} placeholder="provider.py --port 8080" />
          </div>
        </div>
      )}

      <div className="space-y-3 rounded-[var(--radius-sm)] p-4" style={{ background: "var(--bg-secondary)", border: "0.5px solid var(--separator)" }}>
        <div className="flex items-center justify-between">
          <SectionTitle>模型列表</SectionTitle>
          <Btn onClick={addModel}><Plus {...ICON.sm} /> 添加模型</Btn>
        </div>
        {form.models.length === 0 ? (
          <div className="text-[12px]" style={{ color: "var(--fill-tertiary)" }}>暂无模型。点击「添加模型」定义此插件支持的模型。</div>
        ) : (
          <div className="space-y-2">
            {form.models.map((m, idx) => (
              <div key={idx} className="flex items-center gap-2">
                <Input
                  value={m.id}
                  onChange={(v) => updateModel(idx, "id", v)}
                  placeholder="模型 ID"
                  className="flex-1"
                />
                <Input
                  value={m.name}
                  onChange={(v) => updateModel(idx, "name", v)}
                  placeholder="显示名称"
                  className="flex-1"
                />
                <Input
                  value={String(m.contextWindow)}
                  onChange={(v) => updateModel(idx, "contextWindow", parseInt(v) || 0)}
                  placeholder="上下文窗口"
                  type="number"
                  className="w-28"
                />
                <button onClick={() => removeModel(idx)} className="cursor-pointer p-1" style={{ color: "var(--system-red)" }}>
                  <Trash2 {...ICON.sm} />
                </button>
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
