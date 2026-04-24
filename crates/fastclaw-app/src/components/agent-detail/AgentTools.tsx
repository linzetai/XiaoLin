import { ChevronDown } from "lucide-react";
import * as api from "../../lib/api";
import { CollapsibleList, SectionHeader, Toggle } from "./common";
import type { AgentToolInfo } from "../../lib/api";

export function AgentTools({
  fileAccessMode,
  onFileAccessModeChange,
  nonMcpTools,
  filteredTools,
  toolQuery,
  onToolQueryChange,
  onToolToggle,
  togglingTool,
}: {
  fileAccessMode: api.FileAccessMode;
  onFileAccessModeChange: (m: api.FileAccessMode) => void;
  nonMcpTools: AgentToolInfo[];
  filteredTools: AgentToolInfo[];
  toolQuery: string;
  onToolQueryChange: (q: string) => void;
  onToolToggle: (toolId: string, enabled: boolean) => void;
  togglingTool: string | null;
}) {
  return (
    <>
      <div>
        <SectionHeader>文件访问权限</SectionHeader>
        <div className="relative">
          <select
            value={fileAccessMode}
            onChange={(e) => onFileAccessModeChange(e.target.value as api.FileAccessMode)}
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

      <div>
        <SectionHeader count={nonMcpTools.filter((t) => t.enabled).length} total={nonMcpTools.length} searchable query={toolQuery} onQueryChange={onToolQueryChange}>
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
              <Toggle checked={tool.enabled} onChange={(v) => onToolToggle(tool.id, v)} disabled={togglingTool === tool.id} />
            </div>
          )}
        />
      </div>
    </>
  );
}
