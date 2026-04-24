import { AlertTriangle, Trash2 } from "lucide-react";
import { SectionHeader, ListContainer } from "./common";
import { AgentBasicInfo } from "./AgentBasicInfo";
import { AgentIdentity } from "./AgentIdentity";
import { AgentTools } from "./AgentTools";
import { AgentSkills } from "./AgentSkills";
import { ChannelManager } from "./AgentChannels";
import { encodeModelOption, decodeModelOption, useAgentConfigForm } from "./useAgentConfigForm";

export type ConfigSection = "basic" | "tools" | "skills" | "identity";

export function AgentConfigForm({ section }: { section: ConfigSection }) {
  const h = useAgentConfigForm();
  if (!h.agent) return null;

  const {
    activeAgentId,
    agents,
    agent,
    gatewayReady,
    name, setName, models, selectedModel, setSelectedModel, selectedProvider, setSelectedProvider,
    fileAccessMode, setFileAccessMode,
    saving, saveMsg, backendAgent,
    toolQuery, setToolQuery, skillQuery, setSkillQuery,
    handleSave, handleToolToggle, handleSkillToggle, handleDelete,
    handleRefreshSkills, handleUploadSkillFolder, handleUploadSkillZip,
    nonMcpTools, filteredTools, filteredSkills, agentSkills, skillsDeny,
    confirmDelete, setConfirmDelete, deleting,
    togglingTool, togglingSkill,
    refreshingSkills, uploadingSkill, skillMenuOpen, setSkillMenuOpen,
  } = h;

  const effectiveModel = (typeof backendAgent?.model === "string" ? backendAgent.model : backendAgent?.model?.model) ?? agent.model;
  const effectiveProvider = typeof backendAgent?.model === "object" ? backendAgent.model.provider : (selectedProvider ?? "");
  const selectedModelValue = selectedProvider ? encodeModelOption(selectedProvider, selectedModel) : selectedModel;
  const effectiveOptionValue = effectiveProvider ? encodeModelOption(effectiveProvider, effectiveModel) : effectiveModel;
  const isLastAgent = agents.length <= 1;

  return (
    <div className="space-y-5 p-4">
      {section === "basic" && (
        <AgentBasicInfo
          name={name}
          onNameChange={setName}
          models={models}
          selectedModelValue={selectedModelValue}
          onModelSelect={(value) => {
            const parsed = decodeModelOption(value);
            setSelectedModel(parsed.model);
            setSelectedProvider(parsed.provider);
          }}
          encodeModelOption={encodeModelOption}
          effectiveOptionValue={effectiveOptionValue}
          effectiveModel={effectiveModel}
          effectiveProvider={effectiveProvider}
        />
      )}

      {section === "identity" && (
        <AgentIdentity agentId={activeAgentId} ready={gatewayReady} />
      )}

      {section === "tools" && (
        <AgentTools
          fileAccessMode={fileAccessMode}
          onFileAccessModeChange={setFileAccessMode}
          nonMcpTools={nonMcpTools}
          filteredTools={filteredTools}
          toolQuery={toolQuery}
          onToolQueryChange={setToolQuery}
          onToolToggle={handleToolToggle}
          togglingTool={togglingTool}
        />
      )}

      {section === "skills" && (
        <AgentSkills
          agentSkills={agentSkills}
          skillsDeny={skillsDeny}
          filteredSkills={filteredSkills}
          skillQuery={skillQuery}
          onSkillQueryChange={setSkillQuery}
          onRefreshSkills={handleRefreshSkills}
          refreshingSkills={refreshingSkills}
          onSkillToggle={handleSkillToggle}
          togglingSkill={togglingSkill}
          skillMenuOpen={skillMenuOpen}
          onSkillMenuOpen={setSkillMenuOpen}
          onUploadSkillFolder={handleUploadSkillFolder}
          onUploadSkillZip={handleUploadSkillZip}
          uploadingSkill={uploadingSkill}
        />
      )}

      <ChannelManager agentId={activeAgentId} backendAgent={backendAgent} ready={gatewayReady} />

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

      <div className="pt-4" style={{ borderTop: "0.5px solid var(--separator)" }}>
        <SectionHeader>危险操作</SectionHeader>
        {confirmDelete ? (
          <ListContainer>
            <div className="flex items-center gap-3 px-3 py-3">
              <AlertTriangle size={14} strokeWidth={1.5} className="shrink-0" style={{ color: "var(--fill-tertiary)" }} />
              <span className="flex-1 text-[12px]" style={{ color: "var(--fill-secondary)" }}>
                确认删除 &quot;{agent.name}&quot;？此操作不可撤销。
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
