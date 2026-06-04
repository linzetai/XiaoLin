import { useState, useCallback, useRef, useEffect, useMemo, type CSSProperties, type ReactNode } from "react";
import { Plus, Search, Puzzle, RefreshCw, Settings, MessageCircle, Pencil, FolderOpen, Trash2, X } from "lucide-react";
import { createPortal } from "react-dom";
import { useUIStore, MIN_SIDEBAR_WIDTH, MAX_SIDEBAR_WIDTH } from "../../lib/stores";
import { useChatMetaStore } from "../../lib/stores";
import { useGatewayStore } from "../../lib/store";
import { fuzzyMatch } from "../../lib/fuzzy";
import type { ChatMeta } from "../../lib/stores/types";

const actionBtn: CSSProperties = {
  width: "100%",
  borderRadius: 6,
  border: "none",
  background: "transparent",
  color: "var(--fill-tertiary)",
  cursor: "pointer",
  display: "flex",
  alignItems: "center",
  gap: 8,
  padding: "6px 10px",
  fontSize: 13,
  textAlign: "left",
  transition: "background 0.1s, color 0.1s",
};

const ICON_SIZE = 15;

function SidebarAction({ icon, label, onClick, disabled, active }: { icon: ReactNode; label: string; onClick?: () => void; disabled?: boolean; active?: boolean }) {
  const handleClick = useCallback(() => {
    if (disabled) return;
    if (onClick) { onClick(); return; }
  }, [disabled, onClick]);

  const baseStyle: CSSProperties = active
    ? { ...actionBtn, background: "color-mix(in srgb, var(--tint) 10%, transparent)", color: "var(--tint)", fontWeight: 500 }
    : actionBtn;

  return (
    <button
      type="button"
      style={{ ...baseStyle, ...(disabled ? { opacity: 0.5, cursor: "not-allowed" } : {}) }}
      onClick={handleClick}
      onMouseDown={(e) => {
        if (!disabled && !onClick) {
          const el = e.currentTarget;
          el.style.background = "var(--tint)";
          el.style.color = "#fff";
          setTimeout(() => { el.style.background = "transparent"; el.style.color = "var(--fill-tertiary)"; }, 180);
        }
      }}
      onMouseEnter={(e) => { if (!disabled) { e.currentTarget.style.background = active ? "color-mix(in srgb, var(--tint) 15%, transparent)" : "var(--bg-hover)"; e.currentTarget.style.color = active ? "var(--tint)" : "var(--fill-secondary)"; } }}
      onMouseLeave={(e) => {
        if (active) { e.currentTarget.style.background = "color-mix(in srgb, var(--tint) 10%, transparent)"; e.currentTarget.style.color = "var(--tint)"; }
        else { e.currentTarget.style.background = "transparent"; e.currentTarget.style.color = "var(--fill-tertiary)"; }
      }}
    >
      <span style={{ display: "flex", flexShrink: 0 }}>{icon}</span>
      <span>{label}</span>
    </button>
  );
}


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
    const handleKey = (e: KeyboardEvent) => { if (e.key === "Escape") onClose(); };
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
        left: x, top: y,
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
            <Icon size={14} strokeWidth={1.5} />
            {item.label}
          </button>
        );
      })}
    </div>,
    document.body,
  );
}

function extractProjectName(workDir: string): string {
  const parts = workDir.replace(/\\/g, "/").split("/");
  return parts[parts.length - 1] || workDir;
}

function formatTimeAgo(date: Date | string | undefined | null): string {
  if (!date) return "";
  const d = date instanceof Date ? date : new Date(date);
  const now = Date.now();
  const diff = now - d.getTime();
  if (diff < 0 || Number.isNaN(diff)) return "";
  const mins = Math.floor(diff / 60000);
  if (mins < 1) return "now";
  if (mins < 60) return `${mins}m`;
  const hours = Math.floor(mins / 60);
  if (hours < 24) return `${hours}h`;
  const days = Math.floor(hours / 24);
  if (days < 7) return `${days}d`;
  const weeks = Math.floor(days / 7);
  if (weeks < 5) return `${weeks}w`;
  const months = Math.floor(days / 30);
  return `${months}mo`;
}

export function AppSidebar() {
  const collapsed = useUIStore((s) => s.sidebarCollapsed);
  const layoutTier = useUIStore((s) => s.layoutTier);
  const toggleSidebar = useUIStore((s) => s.toggleSidebar);
  const mainView = useUIStore((s) => s.mainView);

  const chats = useChatMetaStore((s) => s.chats);
  const chatOrder = useChatMetaStore((s) => s.chatOrder);
  const activeChatId = useChatMetaStore((s) => s.activeChatId);
  const setActiveChat = useChatMetaStore((s) => s.setActiveChat);
  const newChat = useChatMetaStore((s) => s.newChat);
  const closeChat = useChatMetaStore((s) => s.closeChat);
  const renameChat = useChatMetaStore((s) => s.renameChat);
  const gatewayReady = useGatewayStore((s) => s.connected);

  const chatList = useMemo(
    () => chatOrder.map((id) => chats[id]).filter((c): c is ChatMeta => c != null),
    [chats, chatOrder],
  );

  const [searchOpen, setSearchOpen] = useState(false);
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

  useEffect(() => {
    if (searchOpen) searchInputRef.current?.focus();
  }, [searchOpen]);

  const filteredChats = useMemo(() => {
    if (!query.trim()) return chatList;
    return chatList
      .map((chat) => {
        const result = fuzzyMatch(query, chat.title || "新会话");
        return result ? { chat, score: result.score } : null;
      })
      .filter((r): r is { chat: ChatMeta; score: number } => r !== null)
      .sort((a, b) => b.score - a.score)
      .map((r) => r.chat);
  }, [chatList, query]);

  const handleNewChat = useCallback(() => {
    const activeChat = chatList.find((c) => c.id === activeChatId);
    newChat(activeChat?.workDir ?? undefined);
    useUIStore.getState().setMainView("chat");
  }, [newChat, chatList, activeChatId]);

  const handleSelectChat = useCallback((chatId: string) => {
    setActiveChat(chatId);
    useUIStore.getState().setMainView("chat");
    if (layoutTier === "compact") toggleSidebar();
  }, [setActiveChat, layoutTier, toggleSidebar]);

  const handleRenameSubmit = useCallback(() => {
    if (renamingChatId && renameValue.trim()) {
      renameChat(renamingChatId, renameValue.trim());
    }
    setRenamingChatId(null);
    setRenameValue("");
  }, [renamingChatId, renameValue, renameChat]);

  const groupedChats = useMemo(() => {
    const groups: Record<string, { workDir: string | null; chats: ChatMeta[] }> = {};
    for (const chat of filteredChats) {
      const label = chat.workDir ? extractProjectName(chat.workDir) : "Chats";
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
  }, [filteredChats]);

  const sidebarWidth = useUIStore((s) => s.sidebarWidth);
  const setSidebarWidth = useUIStore((s) => s.setSidebarWidth);
  const resetSidebarWidth = useUIStore((s) => s.resetSidebarWidth);
  const [dragging, setDragging] = useState(false);
  const [resizeHovered, setResizeHovered] = useState(false);

  const handleResizePointerDown = useCallback((e: React.PointerEvent) => {
    e.preventDefault();
    e.stopPropagation();
    (e.target as HTMLElement).setPointerCapture(e.pointerId);
    setDragging(true);
  }, []);

  useEffect(() => {
    if (!dragging) return;
    const handleMove = (e: PointerEvent) => {
      setSidebarWidth(e.clientX);
    };
    const handleUp = () => setDragging(false);
    window.addEventListener("pointermove", handleMove);
    window.addEventListener("pointerup", handleUp);
    document.body.style.cursor = "col-resize";
    document.body.style.userSelect = "none";
    return () => {
      window.removeEventListener("pointermove", handleMove);
      window.removeEventListener("pointerup", handleUp);
      document.body.style.cursor = "";
      document.body.style.userSelect = "";
    };
  }, [dragging, setSidebarWidth]);

  const resolvedWidth = collapsed ? 0 : Math.max(MIN_SIDEBAR_WIDTH, Math.min(MAX_SIDEBAR_WIDTH, sidebarWidth));

  return (
    <>
      <aside
        className="app-sidebar"
        style={{
          width: resolvedWidth,
          minWidth: 0,
          flexShrink: 0,
          display: "flex",
          flexDirection: "column",
          background: "var(--bg-shell)",
          minHeight: 0,
          overflow: "hidden",
          transition: dragging ? "none" : "width 0.2s ease",
          position: "relative",
          pointerEvents: collapsed ? "none" : "auto",
        }}
      >
        {/* Top actions */}
        <div style={{ padding: "10px 8px 6px", display: "flex", flexDirection: "column", gap: 1 }}>
          <SidebarAction
            icon={<Plus size={ICON_SIZE} strokeWidth={1.7} />}
            label="New chat"
            onClick={handleNewChat}
            disabled={!gatewayReady}
          />
          <SidebarAction
            icon={<Search size={ICON_SIZE} strokeWidth={1.7} />}
            label="Search"
            onClick={() => { setSearchOpen(!searchOpen); if (searchOpen) { setQuery(""); } }}
          />
          <SidebarAction icon={<Puzzle size={ICON_SIZE} strokeWidth={1.7} />} label="Plugins" />
          <SidebarAction icon={<RefreshCw size={ICON_SIZE} strokeWidth={1.7} />} label="Automations" onClick={() => useUIStore.getState().setMainView("automations")} active={mainView === "automations"} />
        </div>

        {/* Search bar */}
        {searchOpen && (
          <div style={{ padding: "4px 8px 4px" }}>
            <div
              style={{
                display: "flex",
                alignItems: "center",
                gap: 6,
                height: 32,
                borderRadius: 8,
                padding: "0 8px",
                background: "var(--bg-hover)",
              }}
            >
              <Search size={14} strokeWidth={1.75} style={{ color: "var(--fill-quaternary)", flexShrink: 0 }} />
              <input
                ref={searchInputRef}
                type="text"
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                placeholder="搜索会话..."
                style={{
                  flex: 1,
                  minWidth: 0,
                  background: "transparent",
                  border: "none",
                  outline: "none",
                  fontSize: 13,
                  color: "var(--fill-primary)",
                }}
              />
              {query && (
                <button
                  type="button"
                  onClick={() => setQuery("")}
                  style={{ ...actionBtn, width: 18, height: 18, padding: 0, borderRadius: "50%" }}
                >
                  <X size={12} strokeWidth={2} />
                </button>
              )}
            </div>
          </div>
        )}

        {/* Session list */}
        <div className="sidebar-list" style={{ flex: 1, minHeight: 0, overflowY: "auto", padding: "0 8px 8px" }}>
          {Object.entries(groupedChats).map(([label, { chats: groupChats }]) => (
            <div key={label} style={{ marginBottom: 4 }}>
              <div
                style={{
                  display: "flex",
                  alignItems: "center",
                  justifyContent: "space-between",
                  padding: "12px 10px 4px",
                  fontSize: 11,
                  fontWeight: 500,
                  color: "var(--fill-quaternary)",
                }}
                onMouseEnter={(e) => { const a = e.currentTarget.querySelector(".group-actions") as HTMLElement; if (a) a.style.opacity = "1"; }}
                onMouseLeave={(e) => { const a = e.currentTarget.querySelector(".group-actions") as HTMLElement; if (a) a.style.opacity = "0"; }}
              >
                <span style={{ flex: 1, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                  {label}
                </span>
                <span style={{ display: "flex", gap: 2, opacity: 0, transition: "opacity 0.15s" }} className="group-actions">
                  <button type="button" style={{ background: "none", border: "none", padding: 0, cursor: "pointer", color: "inherit", fontSize: 11, lineHeight: 1 }} title="折叠">⊟</button>
                  <button type="button" style={{ background: "none", border: "none", padding: 0, cursor: "pointer", color: "inherit", fontSize: 11, lineHeight: 1 }} title="排序">≡</button>
                </span>
              </div>
              {groupChats.map((chat) => {
                const active = activeChatId === chat.id;
                const isRenaming = renamingChatId === chat.id;
                const timeLabel = formatTimeAgo(chat.createdAt);
                return (
                  <div
                    key={chat.id}
                    className="group/chat"
                    style={{
                      display: "flex",
                      alignItems: "center",
                      gap: 7,
                      padding: "5px 10px",
                      borderRadius: 6,
                      cursor: "pointer",
                      transition: "background 0.1s",
                      background: active ? "var(--bg-active)" : "transparent",
                      margin: "1px 0",
                    }}
                    onMouseEnter={(e) => { if (!active) e.currentTarget.style.background = "var(--bg-hover)"; }}
                    onMouseLeave={(e) => { if (!active) e.currentTarget.style.background = active ? "var(--bg-active)" : "transparent"; }}
                    onClick={() => !isRenaming && handleSelectChat(chat.id)}
                    onContextMenu={(e) => {
                      e.preventDefault();
                      setContextMenu({ chatId: chat.id, x: e.clientX, y: e.clientY });
                    }}
                  >
                    <span style={{ width: 16, display: "flex", alignItems: "center", justifyContent: "center", flexShrink: 0 }}>
                      <MessageCircle size={14} strokeWidth={1.8} style={{ color: "currentColor" }} />
                    </span>
                    <span style={{
                      flex: 1,
                      minWidth: 0,
                      overflow: "hidden",
                      textOverflow: "ellipsis",
                      whiteSpace: "nowrap",
                      fontSize: 13,
                      fontWeight: active ? 500 : 400,
                      color: active ? "var(--fill-primary)" : "var(--fill-secondary)",
                    }}>
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
                          onClick={(e) => e.stopPropagation()}
                          style={{
                            width: "100%",
                            background: "transparent",
                            border: "none",
                            outline: "none",
                            fontSize: 13,
                            color: "var(--fill-primary)",
                          }}
                        />
                      ) : (
                        chat.title || "新会话"
                      )}
                    </span>
                    {!isRenaming && timeLabel && (
                      <span style={{ fontSize: 11, color: "var(--fill-quaternary)", flexShrink: 0 }}>
                        {timeLabel}
                      </span>
                    )}
                  </div>
                );
              })}
            </div>
          ))}
          {filteredChats.length === 0 && query && (
            <div style={{ padding: "16px 8px", textAlign: "center", fontSize: 12, color: "var(--fill-quaternary)" }}>
              未找到匹配的会话
            </div>
          )}
        </div>

        {/* Bottom: Settings */}
        <div style={{ padding: 8, borderTop: "1px solid var(--border-shell-subtle)" }}>
          <SidebarAction icon={<Settings size={ICON_SIZE} strokeWidth={1.7} />} label="Settings" />
        </div>

        {/* Resize handle */}
        {!collapsed && (
          <div
            style={{
              position: "absolute",
              right: 0,
              top: 0,
              bottom: 0,
              width: 6,
              cursor: "col-resize",
              zIndex: 10,
            }}
            onPointerDown={handleResizePointerDown}
            onDoubleClick={resetSidebarWidth}
            onMouseEnter={() => setResizeHovered(true)}
            onMouseLeave={() => setResizeHovered(false)}
          >
            <div
              style={{
                position: "absolute",
                right: 0,
                top: 0,
                bottom: 0,
                width: 2,
                background: dragging ? "var(--tint)" : "var(--fill-quaternary)",
                opacity: (resizeHovered || dragging) ? (dragging ? 1 : 0.4) : 0,
                transition: "opacity 0.15s",
              }}
            />
          </div>
        )}
      </aside>

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
          onSetWorkDir={() => {/* placeholder */}}
          onDelete={() => closeChat(contextMenu.chatId)}
        />
      )}
    </>
  );
}
