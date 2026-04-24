import { FileText, FolderOpen, RefreshCw, Upload } from "lucide-react";
import * as api from "../../lib/api";
import { CollapsibleList, SectionHeader, Toggle } from "./common";

export function AgentSkills({
  agentSkills,
  skillsDeny,
  filteredSkills,
  skillQuery,
  onSkillQueryChange,
  onRefreshSkills,
  refreshingSkills,
  onSkillToggle,
  togglingSkill,
  skillMenuOpen,
  onSkillMenuOpen,
  onUploadSkillFolder,
  onUploadSkillZip,
  uploadingSkill,
}: {
  agentSkills: api.SkillInfo[];
  skillsDeny: string[];
  filteredSkills: api.SkillInfo[];
  skillQuery: string;
  onSkillQueryChange: (q: string) => void;
  onRefreshSkills: () => void;
  refreshingSkills: boolean;
  onSkillToggle: (skillId: string, enabled: boolean) => void;
  togglingSkill: string | null;
  skillMenuOpen: boolean;
  onSkillMenuOpen: (v: boolean | ((b: boolean) => boolean)) => void;
  onUploadSkillFolder: () => void;
  onUploadSkillZip: () => void;
  uploadingSkill: boolean;
}) {
  return (
    <div>
      <div className="flex items-center justify-between">
        <SectionHeader count={agentSkills.filter((s) => !skillsDeny.includes(s.id)).length} total={agentSkills.length} searchable query={skillQuery} onQueryChange={onSkillQueryChange}>
          Skills
        </SectionHeader>
        <div className="flex items-center gap-1">
          <button
            onClick={onRefreshSkills}
            disabled={refreshingSkills}
            className="cursor-pointer rounded-[var(--radius-xs)] p-1.5 transition-colors duration-100 hover:bg-[var(--bg-hover)] disabled:opacity-40"
            title="刷新 Skills"
          >
            <RefreshCw size={13} strokeWidth={1.5} className={refreshingSkills ? "animate-spin" : ""} style={{ color: "var(--fill-tertiary)" }} />
          </button>
          <div className="relative">
            <button
              onClick={() => onSkillMenuOpen((v) => !v)}
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
                onMouseLeave={() => onSkillMenuOpen(false)}
              >
                <button
                  onClick={() => { onSkillMenuOpen(false); onUploadSkillFolder(); }}
                  className="w-full cursor-pointer px-3 py-2 text-left text-[12px] transition-colors hover:bg-[var(--bg-hover)]"
                  style={{ color: "var(--fill-primary)" }}
                >
                  <FolderOpen size={12} className="mr-2 inline" strokeWidth={1.5} />选择文件夹
                </button>
                <button
                  onClick={() => { onSkillMenuOpen(false); onUploadSkillZip(); }}
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
              <Toggle checked={enabled} onChange={(v) => onSkillToggle(skill.id, v)} disabled={togglingSkill === skill.id} />
            </div>
          );
        }}
      />
    </div>
  );
}
