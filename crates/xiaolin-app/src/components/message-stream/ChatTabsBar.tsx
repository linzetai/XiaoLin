import { useState, useRef, useMemo, useCallback, useEffect } from "react";
import { createPortal } from "react-dom";
import { X, Plus, ChevronDown, MessageSquare } from "lucide-react";
import type { Chat } from "../../lib/agent-store";
import { ICON } from "../../lib/ui-tokens";

export interface ChatTabsBarProps {
  agentId: string;
  chats: Chat[];
  activeChatId: string;
  streamingChatIds: Set<string>;
  attentionChatIds?: Set<string>;
  onSelect: (id: string) => void;
  onClose: (id: string) => void;
  onNew: () => void;
  onRename: (id: string, title: string) => void;
  onReorder: (fromIdx: number, toIdx: number) => void;
}

function SwitcherItem({
  chat, isActive, isStreaming, needsAttention,
  editingId, editValue, editRef,
  onSelect, onClose, onDoubleClick,
  onEditChange, onCommitRename, onCancelEdit,
}: {
  chat: Chat; isActive: boolean; isStreaming: boolean; needsAttention: boolean;
  editingId: string | null; editValue: string;
  editRef: React.RefObject<HTMLInputElement | null>;
  onSelect: (id: string) => void; onClose: (id: string) => void;
  onDoubleClick: (chat: Chat) => void;
  onEditChange: (v: string) => void; onCommitRename: () => void; onCancelEdit: () => void;
}) {
  const [hovered, setHovered] = useState(false);
  const isEditing = editingId === chat.id;

  return (
    <div
      className="group flex items-center gap-2 rounded-md px-2.5 py-1.5 text-[12px]"
      style={{
        background: isActive ? "var(--tint-bg)" : hovered ? "var(--bg-hover)" : "transparent",
        cursor: isEditing ? "text" : "pointer",
        transition: "background var(--duration-fast) var(--ease-in-out)",
      }}
      onClick={() => { if (!isEditing) onSelect(chat.id); }}
      onDoubleClick={() => onDoubleClick(chat)}
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
    >
      {needsAttention && (
        <span
          className="inline-block h-[6px] w-[6px] shrink-0 rounded-full"
          style={{ background: "var(--warning, #f59e0b)", animation: "pulse-subtle 1.5s ease-in-out infinite" }}
          title="需要操作"
        />
      )}
      {!needsAttention && isStreaming && (
        <span
          className="inline-block h-[6px] w-[6px] shrink-0 rounded-full"
          style={{ background: "var(--tint)", animation: "pulse-subtle 1.5s ease-in-out infinite" }}
        />
      )}
      {!needsAttention && !isStreaming && <MessageSquare {...ICON.sm} style={{ color: "var(--fill-quaternary)", flexShrink: 0 }} />}

      {isEditing ? (
        <input
          ref={editRef}
          value={editValue}
          onChange={(e) => onEditChange(e.target.value)}
          onBlur={onCommitRename}
          onKeyDown={(e) => {
            if (e.key === "Enter") onCommitRename();
            if (e.key === "Escape") onCancelEdit();
          }}
          className="min-w-0 flex-1 rounded-sm bg-transparent px-0.5 text-[12px] font-medium outline-none ring-1 ring-[var(--tint)]"
          style={{ color: "var(--fill-primary)" }}
          onClick={(e) => e.stopPropagation()}
        />
      ) : (
        <span
          className="min-w-0 flex-1 truncate"
          style={{ color: isActive ? "var(--fill-primary)" : "var(--fill-secondary)", fontWeight: isActive ? 600 : 400 }}
        >
          {chat.title}
        </span>
      )}

      {chat.workDir && (
        <span className="hidden shrink-0 truncate text-[10px] group-hover:inline" style={{ color: "var(--fill-quaternary)", maxWidth: 120 }}>
          {chat.workDir.replace(/^\/home\/[^/]+\//, "~/")}
        </span>
      )}

      <button
        onClick={(e) => { e.stopPropagation(); onClose(chat.id); }}
        className="flex h-4 w-4 shrink-0 items-center justify-center rounded-sm transition-opacity"
        style={{ color: "var(--fill-tertiary)", opacity: hovered || isActive ? 1 : 0 }}
      >
        <X {...ICON.sm} />
      </button>
    </div>
  );
}

export function ChatTabsBar({ chats, activeChatId, streamingChatIds, attentionChatIds, onSelect, onClose, onNew, onRename, onReorder: _onReorder }: ChatTabsBarProps) {
  const [open, setOpen] = useState(false);
  const [editingId, setEditingId] = useState<string | null>(null);
  const [editValue, setEditValue] = useState("");
  const editRef = useRef<HTMLInputElement>(null);
  const dropdownRef = useRef<HTMLDivElement>(null);
  const portalRef = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);

  const openChats = useMemo(() => chats.filter((c) => c.open), [chats]);
  const activeChat = useMemo(() => openChats.find((c) => c.id === activeChatId), [openChats, activeChatId]);
  const hasMultiple = openChats.length > 1;
  const hasStreamingOther = useMemo(
    () => openChats.some((c) => c.id !== activeChatId && streamingChatIds.has(c.id)),
    [openChats, activeChatId, streamingChatIds],
  );
  const hasAttentionOther = useMemo(
    () => attentionChatIds ? openChats.some((c) => c.id !== activeChatId && attentionChatIds.has(c.id)) : false,
    [openChats, activeChatId, attentionChatIds],
  );

  useEffect(() => {
    if (!open) return;
    const handler = (e: MouseEvent) => {
      const target = e.target as Node;
      if (dropdownRef.current?.contains(target)) return;
      if (portalRef.current?.contains(target)) return;
      setOpen(false);
    };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, [open]);

  useEffect(() => {
    if (!open) return;
    const handler = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    document.addEventListener("keydown", handler);
    return () => document.removeEventListener("keydown", handler);
  }, [open]);

  const handleDblClick = useCallback((chat: Chat) => {
    setEditingId(chat.id);
    setEditValue(chat.title);
    setTimeout(() => editRef.current?.select(), 0);
  }, []);

  const commitRename = useCallback(() => {
    if (editingId && editValue.trim()) onRename(editingId, editValue.trim());
    setEditingId(null);
  }, [editingId, editValue, onRename]);

  const cancelEdit = useCallback(() => setEditingId(null), []);

  const handleSelect = useCallback((id: string) => {
    onSelect(id);
    setOpen(false);
  }, [onSelect]);

  const handleNew = useCallback(() => {
    onNew();
    setOpen(false);
  }, [onNew]);

  const dropdownPos = useMemo(() => {
    if (!open || !triggerRef.current) return { top: 0, left: 0 };
    const rect = triggerRef.current.getBoundingClientRect();
    return { top: rect.bottom + 4, left: rect.left };
  }, [open]);

  return (
    <div className="relative flex shrink-0 items-center" ref={dropdownRef}>
      <button
        ref={triggerRef}
        onClick={() => hasMultiple && setOpen(!open)}
        className="flex items-center gap-1.5 rounded-md px-2.5 py-1 text-[12px] transition-all duration-150 hover:bg-[var(--bg-hover)]"
        style={{
          color: "var(--fill-secondary)",
          cursor: hasMultiple ? "pointer" : "default",
          background: open ? "var(--bg-hover)" : "transparent",
        }}
      >
        <span className="max-w-[180px] truncate font-semibold" style={{ color: "var(--fill-primary)" }}>
          {activeChat?.title ?? "新对话"}
        </span>
        {hasAttentionOther && (
          <span
            className="inline-block h-[5px] w-[5px] rounded-full"
            style={{ background: "var(--warning, #f59e0b)", animation: "pulse-subtle 1.5s ease-in-out infinite" }}
            title="后台会话需要操作"
          />
        )}
        {!hasAttentionOther && hasStreamingOther && (
          <span
            className="inline-block h-[5px] w-[5px] rounded-full"
            style={{ background: "var(--tint)", animation: "pulse-subtle 1.5s ease-in-out infinite" }}
          />
        )}
        {hasMultiple && <ChevronDown {...ICON.sm} style={{ color: "var(--fill-tertiary)", transform: open ? "rotate(180deg)" : "none", transition: "transform 0.15s" }} />}
      </button>

      <button
        onClick={handleNew}
        className="ml-1 flex h-[26px] items-center gap-1 rounded-md px-2 text-[11px] font-medium transition-all duration-150 hover:bg-[var(--tint-bg)] active:scale-95"
        style={{ color: "var(--tint)", border: "0.5px solid var(--border-subtle)" }}
        title="新建会话"
      >
        <Plus {...ICON.sm} />
        <span>新对话</span>
      </button>

      {open && createPortal(
        <div
          ref={portalRef}
          className="fixed min-w-[240px] max-w-[320px] rounded-lg p-1"
          style={{
            top: dropdownPos.top,
            left: dropdownPos.left,
            zIndex: 9999,
            background: "var(--bg-elevated)",
            border: "0.5px solid var(--border-subtle)",
            boxShadow: "var(--shadow-lg), inset 0 1px 0 var(--highlight-top)",
            animation: "scale-spring var(--duration-normal) var(--ease-spring-subtle)",
            transformOrigin: "top left",
          }}
        >
          <button
            onClick={handleNew}
            className="flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-[12px] font-medium transition-colors hover:bg-[var(--bg-hover)]"
            style={{ color: "var(--tint)" }}
          >
            <Plus {...ICON.sm} /> 新建会话
          </button>
          <div className="my-1 h-px" style={{ background: "var(--separator)" }} />
          <div className="max-h-[240px] overflow-y-auto">
            {openChats.map((chat) => (
              <SwitcherItem
                key={chat.id}
                chat={chat}
                isActive={chat.id === activeChatId}
                isStreaming={streamingChatIds.has(chat.id)}
                needsAttention={attentionChatIds?.has(chat.id) ?? false}
                editingId={editingId}
                editValue={editValue}
                editRef={editRef}
                onSelect={handleSelect}
                onClose={onClose}
                onDoubleClick={handleDblClick}
                onEditChange={setEditValue}
                onCommitRename={commitRename}
                onCancelEdit={cancelEdit}
              />
            ))}
          </div>
        </div>,
        document.body,
      )}
    </div>
  );
}
