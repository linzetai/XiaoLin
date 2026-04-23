import { useState, useCallback, useRef, useEffect } from "react";
import { createPortal } from "react-dom";
import { useAgentStore } from "../../lib/agent-store";
import { useGatewayStore } from "../../lib/store";
import { Search, Plus, ChevronDown, Check, Camera, X } from "lucide-react";
import * as api from "../../lib/api";
import * as transport from "../../lib/transport";

export function AgentList() {
  const agents = useAgentStore((s) => s.agents);
  const activeAgentId = useAgentStore((s) => s.activeAgentId);
  const agentChats = useAgentStore((s) => s.agentChats);
  const setActiveAgent = useAgentStore((s) => s.setActiveAgent);
  const syncAgents = useAgentStore((s) => s.syncAgentsFromBackend);
  const toggleDetail = useAgentStore((s) => s.toggleDetail);
  const gatewayReady = useGatewayStore((s) => s.connected);
  const [query, setQuery] = useState("");
  const [creating, setCreating] = useState(false);
  const [showNewForm, setShowNewForm] = useState(false);
  const [newName, setNewName] = useState("");
  const [newAgentId, setNewAgentId] = useState("");
  const [agentIdTouched, setAgentIdTouched] = useState(false);
  const [newModel, setNewModel] = useState("");
  const [newAvatarPath, setNewAvatarPath] = useState<string | null>(null);
  const [newAvatarPreview, setNewAvatarPreview] = useState<string | null>(null);
  const [models, setModels] = useState<api.ModelInfo[]>([]);
  const [modelsLoading, setModelsLoading] = useState(false);
  const [onboardingError, setOnboardingError] = useState<string | null>(null);
  const newInputRef = useRef<HTMLInputElement>(null);

  const refreshModels = useCallback(async () => {
    if (!gatewayReady) return;
    try {
      const m = await api.listModels();
      setModels(m);
      setNewModel((prev) => {
        if (m.length === 0) return "";
        if (!prev || !m.some((item) => item.model === prev)) return m[0].model;
        return prev;
      });
    } catch {
      setModels([]);
      setNewModel("");
    }
  }, [gatewayReady]);

  useEffect(() => {
    if (showNewForm) {
      newInputRef.current?.focus();
      if (gatewayReady) {
        setModelsLoading(true);
        refreshModels().finally(() => {
          setModelsLoading(false);
        });
      }
    }
  }, [showNewForm, gatewayReady, refreshModels]);

  useEffect(() => {
    const onModelsUpdated = () => {
      void refreshModels();
    };
    window.addEventListener("fastclaw:models-updated", onModelsUpdated);
    return () => window.removeEventListener("fastclaw:models-updated", onModelsUpdated);
  }, [refreshModels]);

  const pickAvatar = useCallback(async () => {
    if (!transport.isTauri) return;
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const result = await open({
        title: "选择头像",
        filters: [{ name: "图片", extensions: ["png", "jpg", "jpeg", "webp", "gif"] }],
        multiple: false,
      });
      if (result) {
        const path = typeof result === "string" ? result : result;
        setNewAvatarPath(path as string);
        const { convertFileSrc } = await import("@tauri-apps/api/core");
        setNewAvatarPreview(convertFileSrc(path as string));
      }
    } catch { /* user cancelled */ }
  }, []);

  const handleNewAgent = useCallback(async () => {
    const trimmed = newName.trim();
    if (!trimmed) return;
    setCreating(true);
    const selectedModelMeta = models.find((m) => m.model === newModel);
    const explicitId = newAgentId.trim() || undefined;
    const agent = await api.createAgent({
      name: trimmed,
      agentId: explicitId,
      model: newModel || undefined,
      provider: selectedModelMeta?.provider,
    });
    if (agent) {
      if (newAvatarPath) {
        await api.uploadAgentAvatar(agent.agentId, newAvatarPath);
      }
      const newModelStr = typeof agent.model === "string" ? agent.model : agent.model?.model ?? "";
      syncAgents([...agents.map((a) => ({ agentId: a.id, name: a.name, model: a.model })), { agentId: agent.agentId, name: agent.name ?? "", model: newModelStr }]);
      setActiveAgent(agent.agentId);
    }
    setCreating(false);
    setShowNewForm(false);
    setNewName("");
    setNewModel("");
    setNewAvatarPath(null);
    setNewAvatarPreview(null);
  }, [agents, syncAgents, newName, newModel, newAvatarPath, setActiveAgent, models]);

  const handleQuickCreateDefault = useCallback(async () => {
    setOnboardingError(null);
    let availableModels = models;
    if (availableModels.length === 0) {
      try {
        availableModels = await api.listModels();
        setModels(availableModels);
      } catch {
        availableModels = [];
      }
    }
    if (availableModels.length === 0) {
      setOnboardingError("未检测到可用模型，请先在设置里录入模型。");
      toggleDetail();
      return;
    }

    const defaultName = "Main Agent";
    const created = await api.createAgent({
      name: defaultName,
      model: availableModels[0].model,
      provider: availableModels[0].provider,
    });
    if (!created) {
      setOnboardingError("创建默认 Agent 失败，请稍后重试。");
      return;
    }

    const newModelStr =
      typeof created.model === "string"
        ? created.model
        : created.model?.model ?? "";
    syncAgents([
      ...agents.map((a) => ({ agentId: a.id, name: a.name, model: a.model })),
      { agentId: created.agentId, name: created.name ?? defaultName, model: newModelStr },
    ]);
    setActiveAgent(created.agentId);
  }, [agents, models, setActiveAgent, syncAgents, toggleDetail]);

  const cancelNew = useCallback(() => {
    setShowNewForm(false);
    setNewName("");
    setNewAgentId("");
    setAgentIdTouched(false);
    setNewModel("");
    setNewAvatarPath(null);
    setNewAvatarPreview(null);
  }, []);

  const list = agents.filter(
    (a) => !query || a.name.toLowerCase().includes(query.toLowerCase()),
  );

  return (
    <aside
      className="vibrancy flex shrink-0 flex-col"
      style={{
        width: "var(--sidebar-w)",
        background: "var(--bg-sidebar)",
        borderRight: "0.5px solid var(--separator)",
      }}
    >
      {/* Search */}
      <div className="px-4 pb-2 pt-4">
        <div
          className="flex items-center gap-2.5 rounded-[10px] px-3 py-[7px]"
          style={{ background: "var(--bg-hover)" }}
        >
          <Search size={14} strokeWidth={1.5} style={{ color: "var(--fill-tertiary)" }} />
          <input
            type="text"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="搜索"
            className="min-w-0 flex-1 bg-transparent text-[13px] outline-none"
            style={{ color: "var(--fill-primary)" }}
          />
        </div>
      </div>

      {/* List */}
      <div className="flex-1 overflow-y-auto px-3 py-1.5">
        {list.length === 0 && !query && (
          <div
            className="mx-2 mt-3 rounded-[var(--radius-sm)] p-4"
            style={{ background: "var(--bg-hover)", border: "0.5px solid var(--separator)" }}
          >
            <div className="text-[13px] font-semibold" style={{ color: "var(--fill-primary)" }}>
              欢迎使用 FastClaw
            </div>
            <p className="mt-1 text-[12px] leading-5" style={{ color: "var(--fill-tertiary)" }}>
              先录入模型，再创建第一个 Agent。你也可以一键创建默认 Agent 快速开始。
            </p>
            <div className="mt-3 flex gap-2">
              <button
                onClick={handleQuickCreateDefault}
                className="cursor-pointer rounded-[var(--radius-xs)] px-3 py-1.5 text-[12px] font-medium"
                style={{ background: "var(--fill-primary)", color: "var(--fill-inverse)" }}
              >
                一键创建默认 Agent
              </button>
              <button
                onClick={toggleDetail}
                className="cursor-pointer rounded-[var(--radius-xs)] px-3 py-1.5 text-[12px] font-medium"
                style={{ background: "var(--bg-base)", color: "var(--fill-secondary)", border: "0.5px solid var(--separator-opaque)" }}
              >
                去配置模型
              </button>
            </div>
            {onboardingError && (
              <div className="mt-2 text-[11px]" style={{ color: "#ff453a" }}>
                {onboardingError}
              </div>
            )}
          </div>
        )}
        {list.map((agent, i) => {
          const active = activeAgentId === agent.id;
          const ac = agentChats[agent.id];
          const lastMsg = ac?.lastMsg ?? agent.tagline;
          const lastTime = ac?.lastTime;
          const unread = ac?.unread ?? 0;

          return (
            <button
              key={agent.id}
              onClick={() => setActiveAgent(agent.id)}
              className="mb-0.5 flex w-full cursor-pointer items-center gap-3 rounded-[var(--radius-sm)] px-3 py-3 text-left transition-all duration-150 hover:bg-[var(--bg-hover)]"
              style={{
                background: active ? "var(--bg-active)" : "transparent",
                animation: `slide-up 0.3s ease-out ${i * 0.04}s backwards`,
              }}
            >
              {/* Avatar (greyscale) */}
              <div className="relative shrink-0">
                <div
                  className="flex h-[42px] w-[42px] items-center justify-center rounded-full text-[15px] font-semibold"
                  style={{ background: "var(--bg-tertiary)", color: "var(--fill-secondary)" }}
                >
                  {agent.initial}
                </div>
                {agent.online && (
                  <div
                    className="absolute bottom-0 right-0 h-[11px] w-[11px] rounded-full"
                    style={{ background: "var(--fill-tertiary)", border: "2px solid var(--bg-secondary)" }}
                  />
                )}
              </div>

              {/* Text */}
              <div className="min-w-0 flex-1">
                <div className="flex items-baseline justify-between gap-2">
                  <span className="truncate text-[13px] font-semibold" style={{ color: "var(--fill-primary)" }}>
                    {agent.name}
                  </span>
                  {lastTime && (
                    <span className="shrink-0 text-[11px]" style={{ color: "var(--fill-tertiary)" }}>
                      {lastTime}
                    </span>
                  )}
                </div>
                <div className="mt-0.5 flex items-center justify-between gap-2">
                  <span className="truncate text-[12px]" style={{ color: "var(--fill-tertiary)" }}>
                    {lastMsg}
                  </span>
                  {unread > 0 && (
                    <span
                      className="flex h-[18px] min-w-[18px] shrink-0 items-center justify-center rounded-full px-1 text-[11px] font-bold"
                      style={{ background: "var(--fill-primary)", color: "var(--fill-inverse)" }}
                    >
                      {unread}
                    </span>
                  )}
                </div>
              </div>
            </button>
          );
        })}
      </div>

      {/* New Agent Button */}
      <div className="px-3 pb-3 pt-1">
        <button
          onClick={() => setShowNewForm(true)}
          disabled={creating}
          className="flex w-full cursor-pointer items-center justify-center gap-1.5 rounded-[var(--radius-sm)] py-2.5 text-[13px] font-medium transition-all duration-150 hover:bg-[var(--tint-bg)] disabled:opacity-50"
          style={{ color: "var(--fill-secondary)" }}
        >
          <Plus size={13} strokeWidth={2} />
          新建 Agent
        </button>
      </div>

      {/* New Agent Modal — portaled to body to escape vibrancy containing block */}
      {showNewForm && createPortal(
        <div className="fixed inset-0 z-50 flex items-center justify-center" style={{ animation: "fade-in 0.15s ease-out" }}>
          <div className="absolute inset-0" style={{ background: "rgba(0, 0, 0, 0.3)" }} onClick={cancelNew} />
          <div
            className="relative w-full max-w-[380px] overflow-hidden rounded-[var(--radius-md)]"
            style={{
              background: "var(--bg-elevated)",
              boxShadow: "var(--shadow-lg)",
              animation: "scale-in 0.2s ease-out",
              border: "0.5px solid var(--separator)",
            }}
          >
            <div className="flex items-center justify-between px-5 py-3.5" style={{ borderBottom: "0.5px solid var(--separator)" }}>
              <h3 className="text-[14px] font-semibold" style={{ color: "var(--fill-primary)" }}>新建 Agent</h3>
              <button onClick={cancelNew} className="flex h-6 w-6 cursor-pointer items-center justify-center rounded-full transition-colors duration-100 hover:bg-[var(--bg-hover)]" style={{ color: "var(--fill-tertiary)" }}>
                <X size={12} strokeWidth={2} />
              </button>
            </div>

            <div className="space-y-4 px-5 py-4">
              {/* Avatar + Name */}
              <div className="flex items-center gap-3">
                <button
                  type="button"
                  onClick={pickAvatar}
                  className="group relative flex h-12 w-12 shrink-0 cursor-pointer items-center justify-center overflow-hidden rounded-full"
                  style={{ background: "var(--bg-tertiary)" }}
                  title="选择头像"
                >
                  {newAvatarPreview ? (
                    <img src={newAvatarPreview} alt="" className="h-full w-full object-cover" />
                  ) : (
                    <span className="text-[14px] font-semibold" style={{ color: "var(--fill-tertiary)" }}>
                      {newName.trim() ? newName.trim().charAt(0).toUpperCase() : "?"}
                    </span>
                  )}
                  <div className="absolute inset-0 flex items-center justify-center rounded-full opacity-0 transition-opacity duration-100 group-hover:opacity-100" style={{ background: "rgba(0,0,0,0.3)" }}>
                    <Camera size={14} strokeWidth={1.5} color="white" />
                  </div>
                </button>
                <div className="min-w-0 flex-1">
                  <label className="mb-1 block text-[11px] font-medium" style={{ color: "var(--fill-tertiary)" }}>名称</label>
                  <input
                    ref={newInputRef}
                    type="text"
                    value={newName}
                    onChange={(e) => setNewName(e.target.value)}
                    onKeyDown={(e) => {
                      if (e.key === "Enter" && newName.trim()) handleNewAgent();
                      if (e.key === "Escape") cancelNew();
                    }}
                    placeholder="输入 Agent 名称"
                    className="w-full rounded-[var(--radius-xs)] px-3 py-2 text-[13px] outline-none transition-colors focus:ring-1 focus:ring-[var(--fill-quaternary)]"
                    style={{ background: "var(--bg-base)", color: "var(--fill-primary)", border: "0.5px solid var(--separator-opaque)" }}
                    disabled={creating}
                  />
                </div>
              </div>

              {/* Agent ID */}
              <div>
                <label className="mb-1 block text-[11px] font-medium" style={{ color: "var(--fill-tertiary)" }}>Agent ID</label>
                <input
                  type="text"
                  value={agentIdTouched ? newAgentId : (newName.trim().toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, ""))}
                  onChange={(e) => {
                    setAgentIdTouched(true);
                    setNewAgentId(e.target.value.toLowerCase().replace(/[^a-z0-9-_]/g, ""));
                  }}
                  onFocus={() => {
                    if (!agentIdTouched) {
                      setNewAgentId(newName.trim().toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, ""));
                      setAgentIdTouched(true);
                    }
                  }}
                  placeholder="自动生成，或手动输入"
                  className="w-full rounded-[var(--radius-xs)] px-3 py-2 text-[13px] outline-none transition-colors focus:ring-1 focus:ring-[var(--fill-quaternary)]"
                  style={{ background: "var(--bg-base)", color: "var(--fill-secondary)", border: "0.5px solid var(--separator-opaque)", fontFamily: "var(--font-mono, monospace)" }}
                  disabled={creating}
                />
                <span className="mt-0.5 block text-[10px]" style={{ color: "var(--fill-quaternary)" }}>
                  用于工作目录和文件标识，仅限小写字母、数字、连字符
                </span>
              </div>

              {/* Model */}
              <div>
                <label className="mb-1 block text-[11px] font-medium" style={{ color: "var(--fill-tertiary)" }}>模型</label>
                <div className="relative">
                  <select
                    value={newModel}
                    onChange={(e) => setNewModel(e.target.value)}
                    className="w-full cursor-pointer rounded-[var(--radius-xs)] px-3 py-2 pr-8 text-[13px] outline-none transition-colors focus:ring-1 focus:ring-[var(--fill-quaternary)]"
                    style={{ background: "var(--bg-base)", color: "var(--fill-primary)", border: "0.5px solid var(--separator-opaque)", WebkitAppearance: "none", appearance: "none" } as React.CSSProperties}
                    disabled={creating || modelsLoading || models.length === 0}
                  >
                    {modelsLoading && <option value="">加载中...</option>}
                    {!modelsLoading && models.length === 0 && <option value="">暂无可用模型</option>}
                    {models.map((m) => (
                      <option key={`${m.provider}/${m.model}`} value={m.model}>{m.model}</option>
                    ))}
                  </select>
                  <ChevronDown size={10} strokeWidth={2} className="pointer-events-none absolute top-1/2 right-3 -translate-y-1/2" style={{ color: "var(--fill-tertiary)" }} />
                </div>
              </div>
            </div>

            {/* Actions */}
            <div className="flex items-center justify-end gap-2 px-5 py-3.5" style={{ borderTop: "0.5px solid var(--separator)" }}>
              <button
                onClick={cancelNew}
                className="cursor-pointer rounded-[var(--radius-xs)] px-4 py-1.5 text-[12px] font-medium transition-colors duration-100"
                style={{ color: "var(--fill-secondary)" }}
              >
                取消
              </button>
              <button
                onClick={handleNewAgent}
                disabled={creating || !newName.trim()}
                className="flex cursor-pointer items-center gap-1 rounded-[var(--radius-xs)] px-4 py-1.5 text-[12px] font-medium transition-opacity duration-100 hover:opacity-90 disabled:opacity-40"
                style={{ background: "var(--fill-primary)", color: "var(--fill-inverse)" }}
              >
                <Check size={12} strokeWidth={2} />
                {creating ? "创建中..." : "创建"}
              </button>
            </div>
          </div>
        </div>,
        document.body,
      )}
    </aside>
  );
}
