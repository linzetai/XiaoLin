import { useState, useEffect, useCallback, useMemo } from "react";
import { useAgentStore } from "../../lib/agent-store";
import { useGatewayStore } from "../../lib/store";
import * as api from "../../lib/api";

export function encodeModelOption(provider: string, model: string) {
  return `${provider}::${model}`;
}
export function decodeModelOption(value: string) {
  const sep = value.indexOf("::");
  if (sep < 0) return { provider: null as string | null, model: value };
  return { provider: value.slice(0, sep), model: value.slice(sep + 2) };
}

export function useAgentConfigForm() {
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

  return {
    activeAgentId,
    agents,
    agent,
    gatewayReady,
    name,
    setName,
    selectedModel,
    setSelectedModel,
    selectedProvider,
    setSelectedProvider,
    fileAccessMode,
    setFileAccessMode,
    saving,
    saveMsg,
    models,
    agentTools,
    setAgentTools,
    agentSkills,
    skillsDeny,
    setSkillsDeny,
    togglingTool,
    togglingSkill,
    toolQuery,
    setToolQuery,
    skillQuery,
    setSkillQuery,
    confirmDelete,
    setConfirmDelete,
    deleting,
    refreshingSkills,
    uploadingSkill,
    skillMenuOpen,
    setSkillMenuOpen,
    backendAgent,
    setBackendAgent,
    handleSave,
    handleToolToggle,
    handleSkillToggle,
    handleDelete,
    handleRefreshSkills,
    handleUploadSkillZip,
    handleUploadSkillFolder,
    nonMcpTools,
    filteredTools,
    filteredSkills,
  };
}
