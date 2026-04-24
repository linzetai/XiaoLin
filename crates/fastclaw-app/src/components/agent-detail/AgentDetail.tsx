import { useState, useEffect, useCallback } from "react";
import { useAgentStore } from "../../lib/agent-store";
import { X, Camera } from "lucide-react";
import * as api from "../../lib/api";
import * as transport from "../../lib/transport";
import { ChatsTab } from "./AgentChatsTab";
import { CronTab } from "./AgentCronTab";
import { AgentConfigForm, type ConfigSection } from "./AgentConfigForm";

export interface AgentDetailProps {
  open: boolean;
  onClose: () => void;
  agentName: string;
  agentInitial: string;
  agentColor: string;
}

type MainTab = ConfigSection | "chats" | "cron";

const CONFIG_TABS: MainTab[] = ["basic", "tools", "skills", "identity", "chats", "cron"];

function tabLabel(t: MainTab) {
  switch (t) {
    case "basic": return "基础";
    case "tools": return "工具";
    case "skills": return "技能";
    case "identity": return "身份";
    case "chats": return "会话";
    case "cron": return "定时";
  }
}

export function AgentDetail({ open, onClose, agentName, agentInitial, agentColor: _agentColor }: AgentDetailProps) {
  const [tab, setTab] = useState<MainTab>("basic");
  const activeAgentId = useAgentStore((s) => s.activeAgentId);
  const agents = useAgentStore((s) => s.agents);
  const agent = agents.find((a) => a.id === activeAgentId);
  const [avatarPreview, setAvatarPreview] = useState<string | null>(null);

  useEffect(() => {
    setAvatarPreview(null);
  }, [activeAgentId]);

  const handleAvatarClick = useCallback(async () => {
    if (!transport.isTauri) return;
    try {
      const { open: openDialog } = await import("@tauri-apps/plugin-dialog");
      const selected = await openDialog({ filters: [{ name: "Images", extensions: ["png", "jpg", "jpeg", "webp"] }], multiple: false });
      if (selected) {
        const path = typeof selected === "string" ? selected : (selected as { path?: string }).path;
        if (path) {
          const resp = await api.uploadAgentAvatar(activeAgentId, path);
          if (resp) {
            const { convertFileSrc } = await import("@tauri-apps/api/core");
            setAvatarPreview(convertFileSrc(resp));
          }
        }
      }
    } catch { /* silent */ }
  }, [activeAgentId]);

  const avatarSrc = avatarPreview || (agent?.avatar ? (() => { try { return new URL(agent.avatar!).href; } catch { return undefined; } })() : undefined);

  return (
    <aside
      className="flex shrink-0 flex-col overflow-hidden transition-all duration-300 ease-out"
      style={{
        width: open ? 320 : 0,
        opacity: open ? 1 : 0,
        borderLeft: open ? "0.5px solid var(--separator)" : "none",
        background: "var(--bg-secondary)",
      }}
    >
      <div className="flex shrink-0 items-center justify-between gap-2 px-4 py-3.5" style={{ borderBottom: "0.5px solid var(--separator)" }}>
        <div className="flex min-w-0 flex-1 items-center gap-2.5">
          <button
            className="group relative flex h-8 w-8 shrink-0 cursor-pointer items-center justify-center overflow-hidden rounded-full text-[12px] font-semibold"
            style={{ background: "var(--bg-tertiary)", color: "var(--fill-secondary)" }}
            onClick={handleAvatarClick}
            title="修改头像"
          >
            {avatarSrc ? (
              <img src={avatarSrc} alt="" className="h-full w-full object-cover" />
            ) : (
              agentInitial
            )}
            <div className="absolute inset-0 flex items-center justify-center rounded-full opacity-0 transition-opacity duration-100 group-hover:opacity-100" style={{ background: "rgba(0,0,0,0.3)" }}>
              <Camera size={12} strokeWidth={1.5} color="white" />
            </div>
          </button>
          <span className="min-w-0 truncate text-[14px] font-semibold" style={{ color: "var(--fill-primary)" }} title={agentName}>{agentName}</span>
        </div>
        <button onClick={onClose} className="flex h-7 w-7 shrink-0 cursor-pointer items-center justify-center rounded-full transition-colors duration-100 hover:bg-[var(--bg-hover)]" style={{ color: "var(--fill-tertiary)" }} title="关闭面板">
          <X size={14} strokeWidth={1.5} />
        </button>
      </div>

      <div className="flex shrink-0 flex-wrap gap-0.5 px-2 pt-2 pb-1">
        <div className="flex w-full flex-wrap justify-center gap-0.5 rounded-[var(--radius-xs)] p-0.5" style={{ background: "var(--bg-tertiary)" }}>
          {CONFIG_TABS.map((t) => (
            <button
              key={t}
              onClick={() => setTab(t)}
              className="flex min-w-0 flex-1 cursor-pointer rounded-[4px] px-1.5 py-1.5 text-center text-[11px] font-medium transition-all duration-200 sm:px-2 sm:text-[12px]"
              style={{
                background: tab === t ? "var(--bg-elevated)" : "transparent",
                color: tab === t ? "var(--fill-primary)" : "var(--fill-tertiary)",
                boxShadow: tab === t ? "var(--shadow-sm)" : "none",
                minWidth: "2.5rem",
              }}
            >
              {tabLabel(t)}
            </button>
          ))}
        </div>
      </div>

      <div className="flex-1 overflow-y-auto">
        {tab === "chats" ? <ChatsTab /> : tab === "cron" ? <CronTab key={`cron-${activeAgentId}`} /> : (
          <AgentConfigForm key={activeAgentId} section={tab} />
        )}
      </div>
    </aside>
  );
}
