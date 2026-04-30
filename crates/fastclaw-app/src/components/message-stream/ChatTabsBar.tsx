import { memo, useState, useRef, useMemo, useCallback, type DragEvent } from "react";
import { X, MessageSquare } from "lucide-react";
import type { Chat } from "../../lib/agent-store";

export interface ChatTabsBarProps {
  agentId: string;
  chats: Chat[];
  activeChatId: string;
  streamingChatIds: Set<string>;
  onSelect: (id: string) => void;
  onClose: (id: string) => void;
  onNew: () => void;
  onRename: (id: string, title: string) => void;
  onReorder: (fromIdx: number, toIdx: number) => void;
}

interface ChatTabProps {
  chat: Chat;
  isActive: boolean;
  isStreaming: boolean;
  isDropTarget: boolean;
  editingId: string | null;
  editValue: string;
  editRef: React.RefObject<HTMLInputElement | null>;
  onSelect: (id: string) => void;
  onClose: (id: string) => void;
  onDoubleClick: (chat: Chat) => void;
  onEditChange: (value: string) => void;
  onCommitRename: () => void;
  onCancelEdit: () => void;
  onDragStart: (e: DragEvent, chatId: string) => void;
  onDragEnd: (e: DragEvent) => void;
  onDragOver: (e: DragEvent, chatId: string) => void;
  onDragLeave: () => void;
  onDrop: (e: DragEvent, chatId: string) => void;
}

const ChatTab = memo(function ChatTab({
  chat, isActive, isStreaming, isDropTarget,
  editingId, editValue, editRef,
  onSelect, onClose, onDoubleClick,
  onEditChange, onCommitRename, onCancelEdit,
  onDragStart, onDragEnd, onDragOver, onDragLeave, onDrop,
}: ChatTabProps) {
  const [hovered, setHovered] = useState(false);
  const isEditing = editingId === chat.id;

  return (
    <div
      draggable={!isEditing}
      onDragStart={(e) => onDragStart(e, chat.id)}
      onDragEnd={onDragEnd}
      onDragOver={(e) => onDragOver(e, chat.id)}
      onDragLeave={onDragLeave}
      onDrop={(e) => onDrop(e, chat.id)}
      className="relative flex shrink-0 items-center gap-1 rounded-t-lg px-3 text-[12px]"
      style={{
        height: "calc(100% - 2px)",
        marginTop: 2,
        color: isActive ? "var(--fill-primary)" : "var(--fill-tertiary)",
        borderBottom: isActive ? `2px solid var(--tint)` : "2px solid transparent",
        background: isActive ? "var(--bg-primary)" : hovered ? "var(--bg-hover)" : "transparent",
        fontWeight: isActive ? 600 : 400,
        cursor: isEditing ? "text" : "pointer",
        borderLeft: isDropTarget ? `2px solid var(--tint)` : "2px solid transparent",
        transition: "background var(--duration-fast) var(--ease-in-out), color var(--duration-fast) var(--ease-in-out)",
      }}
      title={chat.workDir ? `${chat.title}\n${chat.workDir.replace(/^\/home\/[^/]+\//, "~/")}` : chat.title}
      onClick={() => { if (!isEditing) onSelect(chat.id); }}
      onDoubleClick={() => onDoubleClick(chat)}
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
    >
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
          className="w-[100px] rounded-sm bg-transparent px-0.5 text-[12px] font-medium outline-none ring-1 ring-[var(--tint)]"
          style={{ color: "var(--fill-primary)" }}
          onClick={(e) => e.stopPropagation()}
        />
      ) : (
        <>
          {isStreaming && !isActive && (
            <span
              className="mr-1 inline-block h-[5px] w-[5px] shrink-0 rounded-full"
              style={{ background: "var(--tint)", animation: "cursor-blink 1s step-end infinite" }}
            />
          )}
          <span className="max-w-[120px] truncate select-none">{chat.title}</span>
        </>
      )}
      <button
        onClick={(e) => { e.stopPropagation(); onClose(chat.id); }}
        className="ml-0.5 flex h-4 w-4 items-center justify-center rounded-sm"
        style={{
          color: "var(--fill-quaternary)",
          opacity: hovered ? 1 : 0,
          transition: "opacity var(--duration-instant) var(--ease-in-out)",
        }}
      >
        <X size={8} strokeWidth={2.5} />
      </button>
    </div>
  );
});

export function ChatTabsBar({ chats, activeChatId, streamingChatIds, onSelect, onClose, onNew, onRename, onReorder }: ChatTabsBarProps) {
  const [editingId, setEditingId] = useState<string | null>(null);
  const [editValue, setEditValue] = useState("");
  const editRef = useRef<HTMLInputElement>(null);
  const [dragId, setDragId] = useState<string | null>(null);
  const [dropTarget, setDropTarget] = useState<string | null>(null);

  const openChats = useMemo(() => chats.filter((c) => c.open), [chats]);

  const handleDblClick = useCallback((chat: Chat) => {
    setEditingId(chat.id);
    setEditValue(chat.title);
    setTimeout(() => editRef.current?.select(), 0);
  }, []);

  const commitRename = useCallback(() => {
    if (editingId && editValue.trim()) {
      onRename(editingId, editValue.trim());
    }
    setEditingId(null);
  }, [editingId, editValue, onRename]);

  const cancelEdit = useCallback(() => setEditingId(null), []);

  const handleDragStart = useCallback((e: DragEvent, chatId: string) => {
    setDragId(chatId);
    e.dataTransfer.effectAllowed = "move";
    e.dataTransfer.setData("text/plain", chatId);
    if (e.currentTarget instanceof HTMLElement) {
      e.currentTarget.style.opacity = "0.5";
    }
  }, []);

  const handleDragEnd = useCallback((e: DragEvent) => {
    setDragId(null);
    setDropTarget(null);
    if (e.currentTarget instanceof HTMLElement) {
      e.currentTarget.style.opacity = "1";
    }
  }, []);

  const handleDragOver = useCallback((e: DragEvent, chatId: string) => {
    e.preventDefault();
    e.dataTransfer.dropEffect = "move";
    if (chatId !== dragId) setDropTarget(chatId);
  }, [dragId]);

  const handleDragLeave = useCallback(() => setDropTarget(null), []);

  const handleDrop = useCallback((e: DragEvent, targetChatId: string) => {
    e.preventDefault();
    if (!dragId || dragId === targetChatId) return;
    const fromIdx = openChats.findIndex((c) => c.id === dragId);
    const toIdx = openChats.findIndex((c) => c.id === targetChatId);
    if (fromIdx >= 0 && toIdx >= 0) onReorder(fromIdx, toIdx);
    setDragId(null);
    setDropTarget(null);
  }, [dragId, openChats, onReorder]);

  return (
    <div
      className="hide-scrollbar flex shrink-0 items-center gap-0 overflow-x-auto px-1"
      style={{
        background: "var(--bg-secondary)",
        borderBottom: `0.5px solid var(--separator)`,
        height: 36,
      }}
    >
      {openChats.map((chat) => (
        <ChatTab
          key={chat.id}
          chat={chat}
          isActive={chat.id === activeChatId}
          isStreaming={streamingChatIds.has(chat.id)}
          isDropTarget={dropTarget === chat.id}
          editingId={editingId}
          editValue={editValue}
          editRef={editRef}
          onSelect={onSelect}
          onClose={onClose}
          onDoubleClick={handleDblClick}
          onEditChange={setEditValue}
          onCommitRename={commitRename}
          onCancelEdit={cancelEdit}
          onDragStart={handleDragStart}
          onDragEnd={handleDragEnd}
          onDragOver={handleDragOver}
          onDragLeave={handleDragLeave}
          onDrop={handleDrop}
        />
      ))}
      <button
        onClick={onNew}
        className="ml-1 flex h-6 w-6 shrink-0 items-center justify-center rounded-md transition-colors duration-100 hover:bg-[var(--bg-hover)]"
        style={{ color: "var(--fill-quaternary)" }}
        title="新建会话"
      >
        <MessageSquare size={12} strokeWidth={1.5} />
      </button>
    </div>
  );
}
