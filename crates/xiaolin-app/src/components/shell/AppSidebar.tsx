import { useState, useCallback, useRef, useEffect, useMemo, type CSSProperties, type ReactNode } from "react";
import { Plus, MagnifyingGlass, PuzzlePiece, ArrowsClockwise, Gear, ChatCircle, PencilSimple, FolderOpen, Trash, CaretRight, CaretDown, PushPin, PushPinSlash, Archive, Palette, FolderPlus } from "@phosphor-icons/react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import { useUIStore, MIN_SIDEBAR_WIDTH, MAX_SIDEBAR_WIDTH, useProjectStore, useSearchStore } from "../../lib/stores";
import { useChatMetaStore } from "../../lib/stores";
import { SearchPanel } from "./SearchPanel";
import { useGatewayStore } from "../../lib/store";
import type { ChatMeta } from "../../lib/stores/types";
import type { ProjectSummary } from "../../lib/transport";

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

function SidebarAction({ icon, label, onClick, disabled }: { icon: ReactNode; label: string; onClick?: () => void; disabled?: boolean }) {
  const handleClick = useCallback(() => {
    if (disabled) return;
    if (onClick) { onClick(); return; }
  }, [disabled, onClick]);

  return (
    <button
      type="button"
      style={{ ...actionBtn, ...(disabled ? { opacity: 0.5, cursor: "not-allowed" } : {}) }}
      onClick={handleClick}
      onMouseEnter={(e) => { if (!disabled) { e.currentTarget.style.background = "var(--bg-hover)"; e.currentTarget.style.color = "var(--fill-secondary)"; } }}
      onMouseLeave={(e) => { e.currentTarget.style.background = "transparent"; e.currentTarget.style.color = "var(--fill-tertiary)"; }}
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
  const { t } = useTranslation("sidebar");
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
    { icon: PencilSimple, label: t("rename"), action: onRename },
    { icon: FolderOpen, label: t("setWorkDir"), action: onSetWorkDir },
    { icon: Trash, label: t("delete"), action: onDelete, danger: true },
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
            <Icon />
            {item.label}
          </button>
        );
      })}
    </div>,
    document.body,
  );
}

const PROJECT_COLORS = ["#2563EB", "#7C3AED", "#EC4899", "#EF4444", "#F97316", "#EAB308", "#22C55E", "#06B6D4"];

function ProjectContextMenu({
  x, y, project, onClose, onRename, onTogglePin, onArchive, onDelete, onChangeColor,
}: {
  x: number; y: number;
  project: ProjectSummary;
  onClose: () => void;
  onRename: () => void;
  onTogglePin: () => void;
  onArchive: () => void;
  onDelete: () => void;
  onChangeColor: (color: string) => void;
}) {
  const { t } = useTranslation("sidebar");
  const ref = useRef<HTMLDivElement>(null);
  const [showColors, setShowColors] = useState(false);

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

  const menuItems = [
    { icon: PencilSimple, label: t("rename"), action: onRename },
    { icon: Palette, label: t("changeColor"), action: () => setShowColors(!showColors) },
    { icon: project.pinned ? PushPinSlash : PushPin, label: project.pinned ? t("pinned") : t("unpinned"), action: onTogglePin },
    { icon: Archive, label: t("archive"), action: onArchive },
    { icon: Trash, label: t("removeFromList"), action: onDelete, danger: true },
  ];

  return createPortal(
    <div
      ref={ref}
      className="fixed z-[60] min-w-[160px] overflow-hidden rounded-lg py-1"
      style={{
        left: x, top: y,
        background: "var(--bg-elevated)",
        border: "0.5px solid var(--separator)",
        boxShadow: "var(--shadow-lg)",
        animation: "scale-in var(--duration-fast) var(--ease-out)",
        transformOrigin: "top left",
      }}
    >
      {menuItems.map((item) => {
        const Icon = item.icon;
        return (
          <button
            key={item.label}
            onClick={() => { item.action(); if (item.label !== t("changeColor")) onClose(); }}
            className="flex w-full items-center gap-2.5 px-3 py-2 text-left text-[12px] font-medium transition-colors duration-100 hover:bg-[var(--bg-hover)]"
            style={{ color: item.danger ? "var(--red)" : "var(--fill-secondary)" }}
          >
            <Icon />
            {item.label}
          </button>
        );
      })}
      {showColors && (
        <div style={{ padding: "6px 10px", display: "grid", gridTemplateColumns: "repeat(4, 1fr)", gap: 6 }}>
          {PROJECT_COLORS.map((color) => (
            <button
              key={color}
              type="button"
              onClick={() => { onChangeColor(color); onClose(); }}
              style={{
                width: 24, height: 24, borderRadius: "50%", border: "none",
                background: color, cursor: "pointer",
                outline: project.color === color ? "2px solid var(--fill-primary)" : "none",
                outlineOffset: 2,
                transition: "transform 0.1s",
              }}
              onMouseEnter={(e) => { e.currentTarget.style.transform = "scale(1.15)"; }}
              onMouseLeave={(e) => { e.currentTarget.style.transform = "scale(1)"; }}
            />
          ))}
        </div>
      )}
    </div>,
    document.body,
  );
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

function SessionItem({
  chat, active, isRenaming, renameValue, renameInputRef,
  onSelect, onContextMenu, onRenameChange, onRenameSubmit, onRenameCancel,
  indent,
}: {
  chat: ChatMeta;
  active: boolean;
  isRenaming: boolean;
  renameValue: string;
  renameInputRef: React.RefObject<HTMLInputElement | null>;
  onSelect: () => void;
  onContextMenu: (e: React.MouseEvent) => void;
  onRenameChange: (v: string) => void;
  onRenameSubmit: () => void;
  onRenameCancel: () => void;
  indent?: boolean;
}) {
  const { t } = useTranslation("sidebar");
  const timeLabel = formatTimeAgo(chat.createdAt);
  return (
    <div
      className="group/chat"
      style={{
        display: "flex",
        alignItems: "center",
        gap: 7,
        padding: "5px 10px",
        paddingLeft: indent ? 34 : 10,
        borderRadius: 6,
        cursor: "pointer",
        transition: "background 0.1s",
        background: active ? "var(--bg-active)" : "transparent",
        margin: "1px 0",
      }}
      onMouseEnter={(e) => { if (!active) e.currentTarget.style.background = "var(--bg-hover)"; }}
      onMouseLeave={(e) => { if (!active) e.currentTarget.style.background = "transparent"; }}
      onClick={() => !isRenaming && onSelect()}
      onContextMenu={onContextMenu}
    >
      <span style={{ width: 16, display: "flex", alignItems: "center", justifyContent: "center", flexShrink: 0 }}>
        <ChatCircle style={{ color: "currentColor" }} />
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
            onChange={(e) => onRenameChange(e.target.value)}
            onBlur={onRenameSubmit}
            onKeyDown={(e) => {
              if (e.key === "Enter") onRenameSubmit();
              if (e.key === "Escape") onRenameCancel();
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
          chat.title || t("newChat")
        )}
      </span>
      {!isRenaming && timeLabel && (
        <span style={{ fontSize: 11, color: "var(--fill-quaternary)", flexShrink: 0 }}>
          {timeLabel}
        </span>
      )}
    </div>
  );
}

function ProjectGroup({
  project, sessions, activeChatId, collapsed, onToggle,
  onSelectChat, onNewChatInProject, onContextMenuChat, onContextMenuProject,
  renamingChatId, renameValue, renameInputRef,
  onRenameChange, onRenameSubmit, onRenameCancel,
}: {
  project: ProjectSummary;
  sessions: ChatMeta[];
  activeChatId: string;
  collapsed: boolean;
  onToggle: () => void;
  onSelectChat: (id: string) => void;
  onNewChatInProject: () => void;
  onContextMenuChat: (chatId: string, e: React.MouseEvent) => void;
  onContextMenuProject: (e: React.MouseEvent) => void;
  renamingChatId: string | null;
  renameValue: string;
  renameInputRef: React.RefObject<HTMLInputElement | null>;
  onRenameChange: (v: string) => void;
  onRenameSubmit: () => void;
  onRenameCancel: () => void;
}) {
  const { t } = useTranslation("sidebar");
  const [hovered, setHovered] = useState(false);
  const Chevron = collapsed ? CaretRight : CaretDown;

  return (
    <div style={{ marginBottom: 2 }}>
      <div
        style={{
          display: "flex",
          alignItems: "center",
          gap: 7,
          padding: "6px 10px",
          borderRadius: 6,
          cursor: "pointer",
          transition: "background 0.1s",
        }}
        onMouseEnter={(e) => { e.currentTarget.style.background = "var(--bg-hover)"; setHovered(true); }}
        onMouseLeave={(e) => { e.currentTarget.style.background = "transparent"; setHovered(false); }}
        onClick={onToggle}
        onContextMenu={onContextMenuProject}
      >
        <span
          style={{
            width: 8, height: 8, borderRadius: "50%",
            background: project.color || "#2563EB",
            flexShrink: 0,
            opacity: project.reachable ? 1 : 0.4,
          }}
        />
        <span style={{
          flex: 1, minWidth: 0,
          overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap",
          fontSize: 13, fontWeight: 500,
          color: project.reachable ? "var(--fill-secondary)" : "var(--fill-quaternary)",
          textDecoration: project.reachable ? "none" : "line-through",
        }}
          title={!project.reachable ? t("projectUnreachable", { path: project.rootPath }) : project.rootPath}
        >
          {project.name}
        </span>
        {hovered && (
          <button
            type="button"
            style={{
              background: "none", border: "none", padding: 0, cursor: "pointer",
              color: "var(--fill-quaternary)", display: "flex", alignItems: "center",
            }}
            title={t("newChatInProject")}
            onClick={(e) => { e.stopPropagation(); onNewChatInProject(); }}
          >
            <Plus size={13} weight="bold" />
          </button>
        )}
        {!hovered && sessions.length > 0 && (
          <span style={{ fontSize: 11, color: "var(--fill-quaternary)", flexShrink: 0 }}>
            {sessions.length}
          </span>
        )}
        <Chevron size={12} style={{
          color: "var(--fill-quaternary)", flexShrink: 0,
          transition: "transform var(--duration-fast) var(--ease-out)",
        }} />
      </div>
      {!collapsed && sessions.map((chat) => (
        <SessionItem
          key={chat.id}
          chat={chat}
          active={activeChatId === chat.id}
          isRenaming={renamingChatId === chat.id}
          renameValue={renameValue}
          renameInputRef={renameInputRef}
          onSelect={() => onSelectChat(chat.id)}
          onContextMenu={(e) => onContextMenuChat(chat.id, e)}
          onRenameChange={onRenameChange}
          onRenameSubmit={onRenameSubmit}
          onRenameCancel={onRenameCancel}
          indent
        />
      ))}
    </div>
  );
}

export function AppSidebar() {
  const { t } = useTranslation("sidebar");
  const collapsed = useUIStore((s) => s.sidebarCollapsed);
  const layoutTier = useUIStore((s) => s.layoutTier);
  const toggleSidebar = useUIStore((s) => s.toggleSidebar);
  const setMainView = useUIStore((s) => s.setMainView);
  const openSettings = useUIStore((s) => s.openSettings);

  const chats = useChatMetaStore((s) => s.chats);
  const chatOrder = useChatMetaStore((s) => s.chatOrder);
  const activeChatId = useChatMetaStore((s) => s.activeChatId);
  const setActiveChat = useChatMetaStore((s) => s.setActiveChat);
  const newChat = useChatMetaStore((s) => s.newChat);
  const closeChat = useChatMetaStore((s) => s.closeChat);
  const renameChat = useChatMetaStore((s) => s.renameChat);
  const gatewayReady = useGatewayStore((s) => s.connected);

  const projects = useProjectStore((s) => s.projects);

  const chatList = useMemo(
    () => chatOrder.map((id) => chats[id]).filter((c): c is ChatMeta => c != null),
    [chats, chatOrder],
  );

  const panelOpen = useSearchStore((s) => s.panelOpen);
  const openSearchPanel = useSearchStore((s) => s.openPanel);
  const [contextMenu, setContextMenu] = useState<{ chatId: string; x: number; y: number } | null>(null);
  const [projectContextMenu, setProjectContextMenu] = useState<{ projectId: string; x: number; y: number } | null>(null);
  const [renamingChatId, setRenamingChatId] = useState<string | null>(null);
  const [renameValue, setRenameValue] = useState("");
  const renameInputRef = useRef<HTMLInputElement>(null);
  const [collapsedProjects, setCollapsedProjects] = useState<Record<string, boolean>>({});

  useEffect(() => {
    if (renamingChatId) {
      renameInputRef.current?.focus();
      renameInputRef.current?.select();
    }
  }, [renamingChatId]);

  useEffect(() => {
    const handleGlobalShortcut = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key === "k") {
        e.preventDefault();
        useSearchStore.getState().openPanel();
      }
    };
    window.addEventListener("keydown", handleGlobalShortcut);
    return () => window.removeEventListener("keydown", handleGlobalShortcut);
  }, []);

  const filteredChats = chatList;

  const { projectGroups, looseChats } = useMemo(() => {
    const groups: Array<{ project: ProjectSummary; sessions: ChatMeta[] }> = [];
    const loose: ChatMeta[] = [];

    for (const chat of filteredChats) {
      if (chat.projectId && projects[chat.projectId]) {
        let group = groups.find((g) => g.project.id === chat.projectId);
        if (!group) {
          group = { project: projects[chat.projectId], sessions: [] };
          groups.push(group);
        }
        group.sessions.push(chat);
      } else {
        loose.push(chat);
      }
    }

    groups.sort((a, b) => {
      if (a.project.pinned !== b.project.pinned) return (b.project.pinned ? 1 : 0) - (a.project.pinned ? 1 : 0);
      return new Date(b.project.lastOpenedAt).getTime() - new Date(a.project.lastOpenedAt).getTime();
    });

    for (const g of groups) {
      g.sessions.sort((a, b) => {
        const ta = a.createdAt instanceof Date ? a.createdAt.getTime() : 0;
        const tb = b.createdAt instanceof Date ? b.createdAt.getTime() : 0;
        return tb - ta;
      });
    }

    loose.sort((a, b) => {
      const ta = a.createdAt instanceof Date ? a.createdAt.getTime() : 0;
      const tb = b.createdAt instanceof Date ? b.createdAt.getTime() : 0;
      return tb - ta;
    });

    return { projectGroups: groups, looseChats: loose };
  }, [filteredChats, projects]);

  const handleNewChat = useCallback(() => {
    newChat();
    setMainView("chat");
  }, [newChat, setMainView]);

  const handleSelectChat = useCallback((chatId: string) => {
    setActiveChat(chatId);
    setMainView("chat");
    if (layoutTier === "compact") toggleSidebar();
  }, [setActiveChat, setMainView, layoutTier, toggleSidebar]);

  const handleRenameSubmit = useCallback(() => {
    if (renamingChatId && renameValue.trim()) {
      renameChat(renamingChatId, renameValue.trim());
    }
    setRenamingChatId(null);
    setRenameValue("");
  }, [renamingChatId, renameValue, renameChat]);

  const handleRenameCancel = useCallback(() => {
    setRenamingChatId(null);
    setRenameValue("");
  }, []);

  const toggleProjectCollapsed = useCallback((projectId: string) => {
    setCollapsedProjects((prev) => ({ ...prev, [projectId]: !prev[projectId] }));
  }, []);

  const handleNewChatInProject = useCallback((rootPath: string) => {
    newChat(rootPath);
  }, [newChat]);

  const createProject = useProjectStore((s) => s.createProject);

  const handleAddProject = useCallback(async () => {
    let selected: string | null = null;
    try {
      const { open: tauriOpenDialog } = await import("@tauri-apps/plugin-dialog");
      selected = await tauriOpenDialog({ directory: true, multiple: false }) as string | null;
    } catch {
      selected = prompt(t("enterProjectPath"));
    }
    if (typeof selected === "string" && selected.trim()) {
      await createProject(selected.trim());
    }
  }, [createProject]);

  const updateProject = useProjectStore((s) => s.updateProject);
  const deleteProjectAction = useProjectStore((s) => s.deleteProject);

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
            icon={<Plus size={ICON_SIZE} />}
            label={t("newChat")}
            onClick={handleNewChat}
            disabled={!gatewayReady}
          />
          <SidebarAction
            icon={<MagnifyingGlass size={ICON_SIZE} />}
            label={t("search")}
            onClick={openSearchPanel}
          />
          <SidebarAction icon={<PuzzlePiece size={ICON_SIZE} />} label={t("plugins")} onClick={() => setMainView("plugins")} />
          <SidebarAction icon={<ArrowsClockwise size={ICON_SIZE} />} label={t("automations")} onClick={() => setMainView("automations")} />
        </div>

        {/* Global search panel or session list */}
        {panelOpen ? (
          <SearchPanel />
        ) : (
        <div className="sidebar-list" style={{ flex: 1, minHeight: 0, overflowY: "auto", paddingLeft: 8, paddingBottom: 8, paddingRight: 2 }}>

          {/* ═══ Projects section ═══ */}
          <div style={{ marginBottom: 8 }}>
            <div style={{
              padding: "12px 10px 4px",
              fontSize: 11, fontWeight: 500, color: "var(--fill-quaternary)",
              display: "flex", alignItems: "center", justifyContent: "space-between",
            }}>
              <span>{t("projects")}</span>
              <button
                type="button"
                title={t("addProject")}
                onClick={handleAddProject}
                style={{
                  background: "none", border: "none", padding: "2px",
                  cursor: "pointer", color: "var(--fill-quaternary)",
                  borderRadius: 4, display: "flex", alignItems: "center",
                  transition: "color 0.1s, background 0.1s",
                }}
                onMouseEnter={(e) => { e.currentTarget.style.color = "var(--fill-secondary)"; e.currentTarget.style.background = "var(--bg-hover)"; }}
                onMouseLeave={(e) => { e.currentTarget.style.color = "var(--fill-quaternary)"; e.currentTarget.style.background = "none"; }}
              >
                <FolderPlus size={13} />
              </button>
            </div>
            {projectGroups.length === 0 && (
              <button
                type="button"
                onClick={handleAddProject}
                style={{
                  display: "flex", alignItems: "center", gap: 6,
                  padding: "8px 10px", margin: "2px 0",
                  width: "100%", border: "1px dashed var(--separator)",
                  borderRadius: 6, background: "transparent",
                  cursor: "pointer", fontSize: 12, color: "var(--fill-quaternary)",
                  transition: "border-color 0.15s, color 0.15s, background 0.15s",
                }}
                onMouseEnter={(e) => { e.currentTarget.style.borderColor = "var(--tint)"; e.currentTarget.style.color = "var(--fill-tertiary)"; e.currentTarget.style.background = "var(--bg-hover)"; }}
                onMouseLeave={(e) => { e.currentTarget.style.borderColor = "var(--separator)"; e.currentTarget.style.color = "var(--fill-quaternary)"; e.currentTarget.style.background = "transparent"; }}
              >
                <FolderOpen size={13} />
                <span>{t("openFolderAsProject")}</span>
              </button>
            )}
            {projectGroups.map(({ project, sessions }) => (
              <ProjectGroup
                key={project.id}
                project={project}
                sessions={sessions}
                activeChatId={activeChatId}
                collapsed={!!collapsedProjects[project.id]}
                onToggle={() => toggleProjectCollapsed(project.id)}
                onSelectChat={handleSelectChat}
                onNewChatInProject={() => handleNewChatInProject(project.rootPath)}
                onContextMenuChat={(chatId, e) => {
                  e.preventDefault();
                  setContextMenu({ chatId, x: e.clientX, y: e.clientY });
                }}
                onContextMenuProject={(e) => {
                  e.preventDefault();
                  setProjectContextMenu({ projectId: project.id, x: e.clientX, y: e.clientY });
                }}
                renamingChatId={renamingChatId}
                renameValue={renameValue}
                renameInputRef={renameInputRef}
                onRenameChange={setRenameValue}
                onRenameSubmit={handleRenameSubmit}
                onRenameCancel={handleRenameCancel}
              />
            ))}
          </div>

          {/* ═══ Chats section ═══ */}
          <div>
            <div style={{
              padding: "12px 10px 4px",
              fontSize: 11, fontWeight: 500, color: "var(--fill-quaternary)",
            }}>
              {t("chats")}
            </div>
            {looseChats.length === 0 && (
              <div style={{ padding: "8px 10px", fontSize: 12, color: "var(--fill-quaternary)" }}>
                {t("noLooseChats")}
              </div>
            )}
            {looseChats.map((chat) => (
              <SessionItem
                key={chat.id}
                chat={chat}
                active={activeChatId === chat.id}
                isRenaming={renamingChatId === chat.id}
                renameValue={renameValue}
                renameInputRef={renameInputRef}
                onSelect={() => handleSelectChat(chat.id)}
                onContextMenu={(e) => {
                  e.preventDefault();
                  setContextMenu({ chatId: chat.id, x: e.clientX, y: e.clientY });
                }}
                onRenameChange={setRenameValue}
                onRenameSubmit={handleRenameSubmit}
                onRenameCancel={handleRenameCancel}
              />
            ))}
          </div>

        </div>
        )}

        {/* Bottom: Settings */}
        <div style={{ padding: 8, borderTop: "1px solid var(--border-shell-subtle)" }}>
          <SidebarAction icon={<Gear size={ICON_SIZE} />} label={t("settings")} onClick={openSettings} />
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
                borderRadius: 1,
                background: dragging ? "var(--tint)" : "var(--fill-quaternary)",
                opacity: (resizeHovered || dragging) ? (dragging ? 1 : 0.6) : 0,
                transition: "opacity 0.15s, background 0.15s",
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
          onSetWorkDir={() => {/* handled by ProjectDropdown in StreamFooter */}}
          onDelete={() => closeChat(contextMenu.chatId)}
        />
      )}

      {projectContextMenu && projects[projectContextMenu.projectId] && (
        <ProjectContextMenu
          x={projectContextMenu.x}
          y={projectContextMenu.y}
          project={projects[projectContextMenu.projectId]}
          onClose={() => setProjectContextMenu(null)}
          onRename={() => {
            const name = prompt(t("projectName"), projects[projectContextMenu.projectId]?.name);
            if (name?.trim()) updateProject(projectContextMenu.projectId, { name: name.trim() });
          }}
          onTogglePin={() => {
            const p = projects[projectContextMenu.projectId];
            if (p) updateProject(p.id, { pinned: !p.pinned });
          }}
          onArchive={() => updateProject(projectContextMenu.projectId, { archived: true })}
          onDelete={() => deleteProjectAction(projectContextMenu.projectId)}
          onChangeColor={(color) => updateProject(projectContextMenu.projectId, { color })}
        />
      )}
    </>
  );
}
