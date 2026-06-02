import { useState, useCallback, useRef, useEffect, useMemo } from "react";
import {
  Search, Plus, X, PanelLeftClose, PanelLeftOpen, MessageCircle,
  MoreHorizontal, Trash2, Pencil, FolderOpen,
} from "lucide-react";
import { createPortal } from "react-dom";
import { ICON, BTN_ICON } from "../../lib/ui-tokens";
import { useAgentStore } from "../../lib/agent-store";
import { useGatewayStore } from "../../lib/store";
import { DEFAULT_AGENT_ID } from "../../lib/stores/chat-helpers";
import { fuzzyMatch } from "../../lib/fuzzy";
import type { Chat } from "../../lib/stores/types";

function ChatContextMenu({
  x, y, onClose, onRename, onSetWorkDir, onDelete,
}: {
  x: number; y: number;
  onClose: () => void;
  onRename: () => void;
  onSetWorkDir: () => void;
  onDelete: () => void;
}) {
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const handleClick = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) onClose();
    };
    const handleKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    document.addEventListener("mousedown", handleClick);
    document.addEventListener("keydown", handleKey);
    return () => {
      document.removeEventListener("mousedown", handleClick);
      document.removeEventListener("keydown", handleKey);
    };
  }, [onClose]);

  const items = [
    { icon: Pencil, label: "重命名", action: onRename },
    { icon: FolderOpen, label: "设置工作目录", action: onSetWorkDir },
    { icon: Trash2, label: "删除", action: onDelete, danger: true },
  ];

  return createPortal(
    <div
      ref={ref}
      className="fixed z-[60] min-w-[140px] overflow-hidden rounded-lg py-1"
      style={{
        left: x,
        top: y,
        background: "var(--bg-elevated)",
        border: "0.5px solid var(--separator)",
        boxShadow: "var(--shadow-lg)",
        animation: "scale-in var(--duration-fast) var(--ease-out)",
        transformOrigin: "top left",
      }}
    >
      {items.map((item) => {
        const Icon = item.icon;
        return (
          <button
            key={item.label}
            onClick={() => { item.action(); onClose(); }}
            className="flex w-full items-center gap-2.5 px-3 py-2 text-left text-[12px] font-medium transition-colors duration-100 hover:bg-[var(--bg-hover)]"
            style={{ color: item.danger ? "var(--red)" : "var(--fill-secondary)" }}
          >
            <Icon {...ICON.md} />
            {item.label}
          </button>
        );
      })}
    </div>,
    document.body,
  );
}

interface SessionListProps {
  collapsed?: boolean;
  onToggleCollapse?: () => void;
}

export function SessionList({ collapsed = false, onToggleCollapse }: SessionListProps) {
  const agentChats = useAgentStore((s) => s.agentChats[DEFAULT_AGENT_ID]);
  const setActiveChat = useAgentStore((s) => s.setActiveChat);
  const newChat = useAgentStore((s) => s.newChat);
  const closeChat = useAgentStore((s) => s.closeChat);
  const renameChat = useAgentStore((s) => s.renameChat);
  const gatewayReady = useGatewayStore((s) => s.connected);

  const chatList = agentChats?.chatList ?? [];
  const activeChatId = agentChats?.activeChatId ?? "";

  const [query, setQuery] = useState("");
  const [contextMenu, setContextMenu] = useState<{ chatId: string; x: number; y: number } | null>(null);
  const [renamingChatId, setRenamingChatId] = useState<string | null>(null);
  const [renameValue, setRenameValue] = useState("");
  const renameInputRef = useRef<HTMLInputElement>(null);
  const searchInputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (renamingChatId) {
      renameInputRef.current?.focus();
      renameInputRef.current?.select();
    }
  }, [renamingChatId]);

  const filteredChats = useMemo(() => {
    if (!query.trim()) return chatList;
    return chatList
      .map((chat) => {
        const result = fuzzyMatch(query, chat.title || "新会话");
        return result ? { chat, score: result.score } : null;
      })
      .filter((r): r is { chat: Chat; score: number } => r !== null)
      .sort((a, b) => b.score - a.score)
      .map((r) => r.chat);
  }, [chatList, query]);

  const handleNewChat = useCallback(() => {
    const activeChat = chatList.find((c) => c.id === activeChatId);
    newChat(DEFAULT_AGENT_ID, activeChat?.workDir ?? undefined);
  }, [newChat, chatList, activeChatId]);

  const handleSelectChat = useCallback((chatId: string) => {
    setActiveChat(DEFAULT_AGENT_ID, chatId);
  }, [setActiveChat]);

  const handleDeleteChat = useCallback((chatId: string) => {
    closeChat(DEFAULT_AGENT_ID, chatId);
  }, [closeChat]);

  const handleRenameSubmit = useCallback(() => {
    if (renamingChatId && renameValue.trim()) {
      renameChat(DEFAULT_AGENT_ID, renamingChatId, renameValue.trim());
    }
    setRenamingChatId(null);
    setRenameValue("");
  }, [renamingChatId, renameValue, renameChat]);

  const extractProjectName = useCallback((workDir: string) => {
    const parts = workDir.replace(/\\/g, "/").split("/");
    return parts[parts.length - 1] || workDir;
  }, []);

  const groupedChats = useMemo(() => {
    const groups: Record<string, { workDir: string | null; chats: Chat[] }> = {};
    for (const chat of filteredChats) {
      const label = chat.workDir
        ? extractProjectName(chat.workDir)
        : "未关联项目";
      if (!groups[label]) groups[label] = { workDir: chat.workDir ?? null, chats: [] };
      groups[label].chats.push(chat);
    }
    for (const group of Object.values(groups)) {
      group.chats.sort((a, b) => {
        const ta = a.createdAt instanceof Date ? a.createdAt.getTime() : 0;
        const tb = b.createdAt instanceof Date ? b.createdAt.getTime() : 0;
        return tb - ta;
      });
    }
    return groups;
  }, [filteredChats, extractProjectName]);

  return (
    <aside
      className="flex shrink-0 flex-col"
      style={{
        width: collapsed ? "0px" : "240px",
        background: "var(--bg-sidebar)",
        borderRight: collapsed ? "none" : "0.5px solid var(--separator)",
        transition: "width var(--duration-slow) var(--ease-in-out)",
        overflow: "hidden",
        opacity: collapsed ? 0 : 1,
        pointerEvents: collapsed ? "none" : "auto",
      }}
      tabIndex={collapsed ? -1 : 0}
    >
      <div className={`flex flex-col gap-2 pb-2 pt-2 ${collapsed ? "items-center px-2" : "px-3"}`}>
        {collapsed ? (
          <button
            onClick={onToggleCollapse}
            className={BTN_ICON.lg}
            style={{ color: "var(--fill-tertiary)" }}
            title="展开侧边栏"
          >
            <PanelLeftOpen size={20} strokeWidth={1.2} />
          </button>
        ) : (
          <>
            <div className="flex items-center gap-1.5">
              <div
                className="flex h-9 min-w-0 flex-1 items-center gap-2 rounded-lg px-3"
                style={{
                  background: "var(--bg-hover)",
                  border: "0.5px solid transparent",
                }}
              >
                <Search {...ICON.sm} style={{ color: "var(--fill-tertiary)", flexShrink: 0 }} />
                <input
                  ref={searchInputRef}
                  type="text"
                  value={query}
                  onChange={(e) => setQuery(e.target.value)}
                  placeholder="搜索会话"
                  className="min-w-0 flex-1 bg-transparent text-[13px] outline-none placeholder:text-[var(--fill-quaternary)]"
                  style={{ color: "var(--fill-primary)" }}
                />
                {query && (
                  <button
                    onClick={() => setQuery("")}
                    className="flex h-4 w-4 shrink-0 cursor-pointer items-center justify-center rounded-full transition-colors duration-150 hover:bg-[var(--bg-active)]"
                    style={{ color: "var(--fill-tertiary)" }}
                  >
                    <X {...ICON.sm} />
                  </button>
                )}
              </div>
              <button
                onClick={onToggleCollapse}
                className={`${BTN_ICON.lg} shrink-0`}
                style={{ color: "var(--fill-tertiary)" }}
                title="折叠侧边栏"
              >
                <PanelLeftClose size={20} strokeWidth={1.2} />
              </button>
            </div>
            <button
              onClick={handleNewChat}
              disabled={!gatewayReady}
              className="flex w-full cursor-pointer items-center justify-center gap-1.5 rounded-lg py-2 text-[12px] font-medium transition-colors duration-150 hover:bg-[var(--bg-hover)] disabled:opacity-50"
              style={{
                color: "var(--fill-tertiary)",
                border: "0.5px dashed var(--separator-opaque)",
              }}
            >
              <Plus {...ICON.sm} />
              新建对话
            </button>
          </>
        )}
      </div>

      <div className={`flex-1 overflow-x-hidden overflow-y-auto py-1 ${collapsed ? "flex flex-col items-center px-2" : "px-1.5"}`}>
        {collapsed ? (
          chatList.filter((c) => c.open).map((chat, i) => (
            <button
              key={chat.id}
              onClick={() => handleSelectChat(chat.id)}
              className="group relative mx-auto mb-1 flex h-9 w-9 items-center justify-center rounded-[var(--radius-xs)] hover:bg-[var(--bg-hover)]"
              style={{
                background: activeChatId === chat.id ? "var(--bg-active)" : "transparent",
                animation: `slide-up var(--duration-slow) var(--ease-out) ${i * 0.04}s backwards`,
              }}
              title={chat.title || "新会话"}
            >
              <MessageCircle {...ICON.md} style={{ color: activeChatId === chat.id ? "var(--tint)" : "var(--fill-quaternary)" }} />
            </button>
          ))
        ) : (
          Object.entries(groupedChats).map(([label, { workDir: groupWorkDir, chats }]) => (
            <div key={label} className="mb-1">
              <div
                className="group/grp flex items-center gap-1.5 px-2.5 pb-1 pt-2 text-[11px] font-medium"
                style={{ color: "var(--fill-quaternary)" }}
              >
                <span className="truncate">{label}</span>
                <span className="shrink-0 opacity-60">{chats.length}</span>
                <button
                  onClick={() => newChat(DEFAULT_AGENT_ID, groupWorkDir ?? undefined)}
                  className="ml-auto shrink-0 rounded p-0.5 opacity-0 transition-opacity duration-100 hover:bg-[var(--bg-hover)] group-hover/grp:opacity-100"
                  style={{ color: "var(--fill-tertiary)" }}
                  title={groupWorkDir ? `在 ${label} 新建对话` : "新建对话"}
                >
                  <Plus size={12} strokeWidth={2} />
                </button>
              </div>
              {chats.map((chat) => {
                const active = activeChatId === chat.id;
                const isRenaming = renamingChatId === chat.id;
                const lastMsg = chat.stream?.length ? chat.stream[chat.stream.length - 1] : null;
                const preview = lastMsg?.type === "message" && lastMsg.data?.content
                  ? lastMsg.data.content.slice(0, 50)
                  : "等待输入...";
                return (
                  <div
                    key={chat.id}
                    className={`group/chat relative flex items-center gap-2.5 rounded-lg px-2.5 py-2 transition-colors duration-100 ${
                      active ? "" : "hover:bg-[var(--bg-hover)]"
                    }`}
                    style={active ? { background: "var(--tint-bg)" } : undefined}
                  >
                    {/* Icon box */}
                    <div
                      className="flex h-[30px] w-[30px] shrink-0 items-center justify-center rounded-lg"
                      style={{
                        background: active ? "var(--tint)" : "var(--bg-secondary)",
                        border: active ? "none" : "0.5px solid var(--separator)",
                      }}
                    >
                      <MessageCircle
                        size={14}
                        strokeWidth={1.5}
                        style={{ color: active ? "#fff" : "var(--fill-quaternary)" }}
                      />
                    </div>
                    {/* Text: title + preview */}
                    <div className="min-w-0 flex-1">
                      {isRenaming ? (
                        <input
                          ref={renameInputRef}
                          type="text"
                          value={renameValue}
                          onChange={(e) => setRenameValue(e.target.value)}
                          onBlur={handleRenameSubmit}
                          onKeyDown={(e) => {
                            if (e.key === "Enter") handleRenameSubmit();
                            if (e.key === "Escape") { setRenamingChatId(null); setRenameValue(""); }
                          }}
                          className="w-full bg-transparent text-[12px] outline-none"
                          style={{ color: "var(--fill-primary)" }}
                        />
                      ) : (
                        <>
                          <button
                            onClick={() => handleSelectChat(chat.id)}
                            className="block w-full cursor-pointer truncate text-left text-[12px] font-medium"
                            style={{ color: active ? "var(--tint)" : "var(--fill-secondary)" }}
                          >
                            {chat.title || "新会话"}
                          </button>
                          <div
                            className="mt-0.5 truncate text-[11px]"
                            style={{ color: "var(--fill-quaternary)" }}
                          >
                            {preview}
                          </div>
                        </>
                      )}
                    </div>
                    {!isRenaming && (
                      <button
                        onClick={(e) => {
                          e.stopPropagation();
                          const rect = (e.target as HTMLElement).getBoundingClientRect();
                          setContextMenu({ chatId: chat.id, x: rect.right, y: rect.bottom });
                        }}
                        className={`${BTN_ICON.sm} shrink-0 opacity-0 transition-opacity duration-100 group-hover/chat:opacity-100`}
                        style={{ color: "var(--fill-tertiary)" }}
                      >
                        <MoreHorizontal {...ICON.sm} />
                      </button>
                    )}
                  </div>
                );
              })}
            </div>
          ))
        )}
        {!collapsed && filteredChats.length === 0 && query && (
          <div className="px-3 py-4 text-center text-[12px]" style={{ color: "var(--fill-quaternary)" }}>
            未找到匹配的会话
          </div>
        )}
      </div>

      {contextMenu && (
        <ChatContextMenu
          x={contextMenu.x}
          y={contextMenu.y}
          onClose={() => setContextMenu(null)}
          onRename={() => {
            const chat = chatList.find((c) => c.id === contextMenu.chatId);
            setRenamingChatId(contextMenu.chatId);
            setRenameValue(chat?.title || "");
          }}
          onSetWorkDir={() => {
            /* placeholder - will open folder picker */
          }}
          onDelete={() => handleDeleteChat(contextMenu.chatId)}
        />
      )}
    </aside>
  );
}
