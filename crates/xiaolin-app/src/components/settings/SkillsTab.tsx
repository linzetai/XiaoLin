import { useState, useEffect, useCallback, useMemo } from "react";
import { useGatewayStore } from "../../lib/store";
import { RefreshCw, Upload, FolderOpen, FileText, Globe, User } from "lucide-react";
import { ICON } from "../../lib/ui-tokens";
import * as api from "../../lib/api";
import { SectionTitle } from "./SettingsShared";


export function SkillsTab() {
  const [publicSkills, setPublicSkills] = useState<api.SkillInfo[]>([]);
  const [agentSkillsMap, setAgentSkillsMap] = useState<Record<string, api.SkillInfo[]>>({});
  const [tools, setTools] = useState<api.ToolInfo[]>([]);
  const [loading, setLoading] = useState(true);
  const [filter, setFilter] = useState<"skills" | "tools">("skills");
  const [refreshing, setRefreshing] = useState(false);
  const [uploading, setUploading] = useState(false);
  const [skillMenuOpen, setSkillMenuOpen] = useState(false);
  const gatewayReady = useGatewayStore((s) => s.connected);

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

  const totalSkills = useMemo(
    () => publicSkills.length + Object.values(agentSkillsMap).reduce((s, a) => s + a.length, 0),
    [publicSkills, agentSkillsMap],
  );

  if (loading) {
    return (
      <div className="flex items-center justify-center py-12">
        <span className="text-[13px]" style={{ color: "var(--fill-tertiary)" }}>加载中...</span>
      </div>
    );
  }

  const SkillRow = ({ skill, isLast }: { skill: api.SkillInfo; isLast: boolean }) => (
    <div
      className="px-4 py-2.5 transition-colors duration-100 hover:bg-[var(--bg-hover)]"
      style={!isLast ? { borderBottom: "0.5px solid var(--separator)" } : undefined}
    >
      <div className="min-w-0">
        <div className="min-w-0 overflow-hidden">
          <div className="flex items-baseline gap-2">
            <span className="break-all text-[13px] font-semibold leading-snug" style={{ color: "var(--fill-primary)" }}>{skill.name}</span>
            {skill.version && <span className="shrink-0 text-[10px]" style={{ color: "var(--fill-quaternary)" }}>v{skill.version}</span>}
          </div>
          {skill.description && (
            <div className="mt-0.5 line-clamp-2 text-[11px] leading-relaxed" style={{ color: "var(--fill-tertiary)" }}>{skill.description}</div>
          )}
          {skill.tags && skill.tags.length > 0 && (
            <div className="mt-1 flex flex-wrap gap-1">
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
    <div className="min-w-0 space-y-4 overflow-hidden">
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
                <RefreshCw {...ICON.sm} className={refreshing ? "animate-spin" : ""} style={{ color: "var(--fill-tertiary)" }} />
              </button>
              <div className="relative">
                <button
                  onClick={() => setSkillMenuOpen((v) => !v)}
                  disabled={uploading}
                  className="cursor-pointer rounded-[var(--radius-xs)] p-1.5 transition-colors duration-100 hover:bg-[var(--bg-hover)] disabled:opacity-40"
                  title="上传 Skill"
                >
                  <Upload {...ICON.sm} style={{ color: "var(--fill-tertiary)" }} />
                </button>
                {skillMenuOpen && (
                  <div
                    className="absolute right-0 top-full z-50 mt-1 min-w-[140px] overflow-hidden rounded-[var(--radius-sm)] py-1 shadow-lg"
                    style={{ background: "var(--bg-elevated)", border: "0.5px solid var(--separator-opaque)" }}
                    onMouseLeave={() => setSkillMenuOpen(false)}
                  >
                    <button
                      onClick={() => { setSkillMenuOpen(false); handleUploadFolder(); }}
                      className="w-full cursor-pointer px-3 py-2 text-left text-[12px] transition-colors hover:bg-[var(--bg-hover)]"
                      style={{ color: "var(--fill-primary)" }}
                    >
                      <FolderOpen {...ICON.sm} className="mr-2 inline" />选择文件夹
                    </button>
                    <button
                      onClick={() => { setSkillMenuOpen(false); handleUploadZip(); }}
                      className="w-full cursor-pointer px-3 py-2 text-left text-[12px] transition-colors hover:bg-[var(--bg-hover)]"
                      style={{ color: "var(--fill-primary)" }}
                    >
                      <FileText {...ICON.sm} className="mr-2 inline" />选择 ZIP 文件
                    </button>
                  </div>
                )}
              </div>
            </div>
          )}
          <div className="flex rounded-[var(--radius-xs)] p-0.5" style={{ background: "var(--bg-tertiary)" }}>
            {(["skills", "tools"] as const).map((f) => (
              <button
                key={f}
                onClick={() => setFilter(f)}
                className="rounded-[var(--radius-xs)] px-2.5 py-1 text-[11px] font-medium transition-all duration-150"
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
                <Globe {...ICON.sm} />
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
                    <User {...ICON.sm} />
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