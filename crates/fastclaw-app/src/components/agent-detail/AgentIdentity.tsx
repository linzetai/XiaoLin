import { useState, useEffect } from "react";
import { ChevronRight, FileText, User, Shield, Wrench } from "lucide-react";
import * as api from "../../lib/api";
import { ListContainer, SectionHeader } from "./common";

const IDENTITY_FILES = [
  { key: "soul" as const, name: "SOUL.md", desc: "人格与语气", icon: <User size={13} strokeWidth={1.5} /> },
  { key: "user" as const, name: "USER.md", desc: "用户画像", icon: <FileText size={13} strokeWidth={1.5} /> },
  { key: "agents" as const, name: "AGENTS.md", desc: "规则与约束", icon: <Shield size={13} strokeWidth={1.5} /> },
  { key: "tools" as const, name: "TOOLS.md", desc: "工具使用指南", icon: <Wrench size={13} strokeWidth={1.5} /> },
] as const;

export function AgentIdentity({ agentId, ready }: { agentId: string; ready: boolean }) {
  const [files, setFiles] = useState<api.IdentityFiles>({ soul: null, user: null, agents: null, tools: null });
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
