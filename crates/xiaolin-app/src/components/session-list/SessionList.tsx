import { useState, useCallback, useRef, useEffect, useMemo } from "react";
import {
  MagnifyingGlass, Plus, X, SidebarSimple, ChatCircle,
  DotsThree, Trash, PencilSimple, FolderOpen,
} from "@phosphor-icons/react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import { ICON_SIZE, BTN_ICON } from "../../lib/ui-tokens";
import { useChatMetaStore, useStreamStore } from "../../lib/stores";
import { useUIStore } from "../../lib/stores";
import { useGatewayStore } from "../../lib/store";
import { fuzzyMatch } from "../../lib/fuzzy";
import type { ChatMeta, StreamItem } from "../../lib/stores/types";

function chatPreview(stream: StreamItem[] | undefined, meta: ChatMeta, waitingText: string): string {
  if (stream?.length) {
    for (let i = stream.length - 1; i >= 0; i--) {
      const item = stream[i];
      if (item.type === "message" && item.data?.content) {
        return item.data.content.slice(0, 60);
      }
    }
  }
  if (meta.messageCount > 0) return meta.title || "";
  return waitingText;
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
  const { t } = useTranslation("sidebar");
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
    { icon: PencilSimple, label: t("rename"), action: onRename },
    { icon: FolderOpen, label: t("setWorkDir"), action: onSetWorkDir },
    { icon: Trash, label: t("delete"), action: onDelete, danger: true },
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
            <Icon size={ICON_SIZE.md} />
            {item.label}
          </button>
        );
      })}
    </div>,
    document.body,
  );
}

function ResizeHandle() {
  const setSidebarWidth = useUIStore((s) => s.setSidebarWidth);
  const resetSidebarWidth = useUIStore((s) => s.resetSidebarWidth);
  const [dragging, setDragging] = useState(false);
  const [hovered, setHovered] = useState(false);

  const handlePointerDown = useCallback((e: React.PointerEvent) => {
    e.preventDefault();
    e.stopPropagation();
    (e.target as HTMLElement).setPointerCapture(e.pointerId);
    setDragging(true);
  }, []);

  useEffect(() => {
    if (!dragging) return;
    const handleMove = (e: PointerEvent) => {
      const navRailW = parseInt(getComputedStyle(document.documentElement).getPropertyValue("--nav-rail-w")) || 54;
      setSidebarWidth(e.clientX - navRailW);
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

  const visible = hovered || dragging;

  return (
    <div
      className="absolute right-0 top-0 bottom-0 z-10"
      style={{ width: 8, cursor: "col-resize" }}
      onPointerDown={handlePointerDown}
      onDoubleClick={resetSidebarWidth}
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
    >
      <div
        className="absolute right-0 top-0 bottom-0 transition-opacity duration-150"
        style={{
          width: 2,
          background: dragging ? "var(--tint)" : "var(--fill-quaternary)",
          opacity: visible ? (dragging ? 1 : 0.5) : 0,
        }}
      />
    </div>
  );
}

interface SessionListProps {
  collapsed?: boolean;
  onToggleCollapse?: () => void;
}

export function SessionList({ collapsed = false, onToggleCollapse }: SessionListProps) {
  const { t } = useTranslation("sidebar");
  const chats = useChatMetaStore((s) => s.chats);
  const chatOrder = useChatMetaStore((s) => s.chatOrder);
  const activeChatId = useChatMetaStore((s) => s.activeChatId);
  const setActiveChat = useChatMetaStore((s) => s.setActiveChat);
  const newChat = useChatMetaStore((s) => s.newChat);
  const closeChat = useChatMetaStore((s) => s.closeChat);
  const renameChat = useChatMetaStore((s) => s.renameChat);
  const streams = useStreamStore((s) => s.streams);
  const gatewayReady = useGatewayStore((s) => s.connected);
  const sidebarWidth = useUIStore((s) => s.sidebarWidth);
  const layoutTier = useUIStore((s) => s.layoutTier);

  const chatList = useMemo(
    () => chatOrder.map((id) => chats[id]).filter((c): c is ChatMeta => c != null),
    [chats, chatOrder],
  );

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
        const result = fuzzyMatch(query, chat.title || t("newChat"));
        return result ? { chat, score: result.score } : null;
      })
      .filter((r): r is { chat: ChatMeta; score: number } => r !== null)
      .sort((a, b) => b.score - a.score)
      .map((r) => r.chat);
  }, [chatList, query, t]);

  const handleNewChat = useCallback(() => {
    const activeChat = chatList.find((c) => c.id === activeChatId);
    newChat(activeChat?.workDir ?? undefined);
  }, [newChat, chatList, activeChatId]);

  const handleSelectChat = useCallback((chatId: string) => {
    setActiveChat(chatId);
    if (layoutTier === "compact" && onToggleCollapse) onToggleCollapse();
  }, [setActiveChat, layoutTier, onToggleCollapse]);

  const handleDeleteChat = useCallback((chatId: string) => {
    closeChat(chatId);
  }, [closeChat]);

  const handleRenameSubmit = useCallback(() => {
    if (renamingChatId && renameValue.trim()) {
      renameChat(renamingChatId, renameValue.trim());
    }
    setRenamingChatId(null);
    setRenameValue("");
  }, [renamingChatId, renameValue, renameChat]);

  const extractProjectName = useCallback((workDir: string) => {
    const parts = workDir.replace(/\\/g, "/").split("/");
    return parts[parts.length - 1] || workDir;
  }, []);

  const groupedChats = useMemo(() => {
    const groups: Record<string, { workDir: string | null; chats: ChatMeta[] }> = {};
    for (const chat of filteredChats) {
      const label = chat.workDir
        ? extractProjectName(chat.workDir)
        : t("unlinkedProject");
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
  }, [filteredChats, extractProjectName, t]);

  const isCompactOverlay = layoutTier === "compact" && !collapsed;

  return (
    <>
    {isCompactOverlay && (
      <div
        className="fixed inset-0 z-30"
        style={{ background: "rgba(0,0,0,0.25)" }}
        onClick={onToggleCollapse}
      />
    )}
    <aside
      className={`flex shrink-0 flex-col${isCompactOverlay ? " fixed left-0 top-0 bottom-0 z-40" : " relative"}`}
      style={{
        width: collapsed ? 0 : sidebarWidth,
        background: "var(--bg-sidebar)",
        borderRight: collapsed ? "none" : "0.5px solid var(--separator)",
        transition: "width var(--duration-slow) var(--ease-in-out)",
        overflow: collapsed ? "visible" : "hidden",
        pointerEvents: collapsed ? "none" : "auto",
        ...(isCompactOverlay ? { boxShadow: "4px 0 16px rgba(0,0,0,0.15)" } : {}),
      }}
      tabIndex={collapsed ? -1 : 0}
    >
      {collapsed && (
        <button
          onClick={onToggleCollapse}
          className="absolute left-1 top-1 z-10 flex h-7 w-7 items-center justify-center rounded-[var(--radius-xs)] transition-colors duration-150 hover:bg-[var(--bg-hover)]"
          style={{ color: "var(--fill-tertiary)", pointerEvents: "auto" }}
          title={t("expandSidebar")}
        >
          <SidebarSimple size={16} weight="light" style={{ transform: "scaleX(-1)" }} />
        </button>
      )}
      {!collapsed && !isCompactOverlay && <ResizeHandle />}
      <div className="flex flex-col gap-2 px-3 pb-2 pt-2">
        <div className="flex items-center gap-1.5">
          <div
            className="flex h-9 min-w-0 flex-1 items-center gap-2 rounded-lg px-3"
            style={{
              background: "var(--bg-hover)",
              border: "0.5px solid transparent",
            }}
          >
            <MagnifyingGlass style={{ color: "var(--fill-tertiary)", flexShrink: 0 }} />
            <input
              ref={searchInputRef}
              type="text"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder={t("searchSessions")}
              className="min-w-0 flex-1 bg-transparent text-[13px] outline-none placeholder:text-[var(--fill-quaternary)]"
              style={{ color: "var(--fill-primary)" }}
            />
            {query && (
              <button
                onClick={() => setQuery("")}
                className="flex h-4 w-4 shrink-0 cursor-pointer items-center justify-center rounded-full transition-colors duration-150 hover:bg-[var(--bg-active)]"
                style={{ color: "var(--fill-tertiary)" }}
              >
                <X />
              </button>
            )}
          </div>
          <button
            onClick={onToggleCollapse}
            className={`${BTN_ICON.lg} shrink-0`}
            style={{ color: "var(--fill-tertiary)" }}
            title={t("collapseSidebar")}
          >
            <SidebarSimple size={20} weight="light" />
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
          <Plus />
          {t("newChatAction")}
        </button>
      </div>

      <div className="flex-1 overflow-x-hidden overflow-y-auto px-1.5 py-1">
        {Object.entries(groupedChats).map(([label, { workDir: groupWorkDir, chats }]) => (
            <div key={label} className="mb-1">
              <div
                className="group/grp flex items-center gap-1.5 px-2.5 pb-1 pt-2 text-[11px] font-medium"
                style={{ color: "var(--fill-quaternary)" }}
              >
                <span className="truncate">{label}</span>
                <span className="shrink-0 opacity-60">{chats.length}</span>
                <button
                  onClick={() => newChat(groupWorkDir ?? undefined)}
                  className="ml-auto shrink-0 rounded p-0.5 opacity-0 transition-opacity duration-100 hover:bg-[var(--bg-hover)] group-hover/grp:opacity-100"
                  style={{ color: "var(--fill-tertiary)" }}
                  title={groupWorkDir ? t("newChatIn", { name: label }) : t("newChatAction")}
                >
                  <Plus size={12} />
                </button>
              </div>
              {chats.map((chat) => {
                const active = activeChatId === chat.id;
                const isRenaming = renamingChatId === chat.id;
                const preview = chatPreview(streams[chat.id], chat, t("waitingForInput"));
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
                      <ChatCircle
                        size={14}
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
                            {chat.title || t("newChat")}
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
                        <DotsThree />
                      </button>
                    )}
                  </div>
                );
              })}
            </div>
          ))}
        {filteredChats.length === 0 && query && (
          <div className="px-3 py-4 text-center text-[12px]" style={{ color: "var(--fill-quaternary)" }}>
            {t("noMatchingSessions")}
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
    </>
  );
}
