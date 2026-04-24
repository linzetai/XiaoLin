import { useState, useRef, type DragEvent } from "react";
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

export function ChatTabsBar({ chats, activeChatId, streamingChatIds, onSelect, onClose, onNew, onRename, onReorder }: ChatTabsBarProps) {
  const [editingId, setEditingId] = useState<string | null>(null);
  const [editValue, setEditValue] = useState("");
  const editRef = useRef<HTMLInputElement>(null);
  const [dragId, setDragId] = useState<string | null>(null);
  const [dropTarget, setDropTarget] = useState<string | null>(null);
  const [hoveredTab, setHoveredTab] = useState<string | null>(null);

  const openChats = chats.filter((c) => c.open);

  const handleDblClick = (chat: Chat) => {
    setEditingId(chat.id);
    setEditValue(chat.title);
    setTimeout(() => editRef.current?.select(), 0);
  };

  const commitRename = () => {
    if (editingId && editValue.trim()) {
      onRename(editingId, editValue.trim());
    }
    setEditingId(null);
  };

  const handleDragStart = (e: DragEvent, chatId: string) => {
    setDragId(chatId);
    e.dataTransfer.effectAllowed = "move";
    e.dataTransfer.setData("text/plain", chatId);
    if (e.currentTarget instanceof HTMLElement) {
      e.currentTarget.style.opacity = "0.5";
    }
  };

  const handleDragEnd = (e: DragEvent) => {
    setDragId(null);
    setDropTarget(null);
    if (e.currentTarget instanceof HTMLElement) {
      e.currentTarget.style.opacity = "1";
    }
  };

  const handleDragOver = (e: DragEvent, chatId: string) => {
    e.preventDefault();
    e.dataTransfer.dropEffect = "move";
    if (chatId !== dragId) setDropTarget(chatId);
  };

  const handleDrop = (e: DragEvent, targetChatId: string) => {
    e.preventDefault();
    if (!dragId || dragId === targetChatId) return;
    const fromIdx = openChats.findIndex((c) => c.id === dragId);
    const toIdx = openChats.findIndex((c) => c.id === targetChatId);
    if (fromIdx >= 0 && toIdx >= 0) onReorder(fromIdx, toIdx);
    setDragId(null);
    setDropTarget(null);
  };

  return (
    <div
      className="hide-scrollbar flex shrink-0 items-center gap-0 overflow-x-auto px-1"
      style={{
        background: "var(--bg-secondary)",
        borderBottom: `0.5px solid var(--separator)`,
        height: 36,
      }}
    >
      {openChats.map((chat) => {
        const isActive = chat.id === activeChatId;
        const isDropHere = dropTarget === chat.id;
        return (
          <div
            key={chat.id}
            draggable={editingId !== chat.id}
            onDragStart={(e) => handleDragStart(e, chat.id)}
            onDragEnd={handleDragEnd}
            onDragOver={(e) => handleDragOver(e, chat.id)}
            onDragLeave={() => setDropTarget(null)}
            onDrop={(e) => handleDrop(e, chat.id)}
            className="relative flex shrink-0 items-center gap-1 rounded-t-lg px-3 text-[12px] transition-all duration-150"
            style={{
              height: "calc(100% - 2px)",
              marginTop: 2,
              color: isActive ? "var(--fill-primary)" : "var(--fill-tertiary)",
              borderBottom: isActive ? `2px solid var(--tint)` : "2px solid transparent",
              background: isActive ? "var(--bg-primary)" : hoveredTab === chat.id ? "var(--bg-hover)" : "transparent",
              fontWeight: isActive ? 600 : 400,
              cursor: editingId === chat.id ? "text" : "pointer",
              borderLeft: isDropHere ? `2px solid var(--tint)` : "2px solid transparent",
            }}
            title={chat.workDir ? `${chat.title}\n${chat.workDir.replace(/^\/home\/[^/]+\//, "~/")}` : chat.title}
            onClick={() => { if (editingId !== chat.id) onSelect(chat.id); }}
            onDoubleClick={() => handleDblClick(chat)}
            onMouseEnter={() => setHoveredTab(chat.id)}
            onMouseLeave={() => setHoveredTab(null)}
          >
            {editingId === chat.id ? (
              <input
                ref={editRef}
                value={editValue}
                onChange={(e) => setEditValue(e.target.value)}
                onBlur={commitRename}
                onKeyDown={(e) => {
                  if (e.key === "Enter") commitRename();
                  if (e.key === "Escape") setEditingId(null);
                }}
                className="w-[100px] rounded-sm bg-transparent px-0.5 text-[12px] font-medium outline-none ring-1 ring-[var(--tint)]"
                style={{ color: "var(--fill-primary)" }}
                onClick={(e) => e.stopPropagation()}
              />
            ) : (
              <>
                {streamingChatIds.has(chat.id) && !isActive && (
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
              className="ml-0.5 flex h-4 w-4 items-center justify-center rounded-sm transition-opacity duration-100"
              style={{
                color: "var(--fill-quaternary)",
                opacity: hoveredTab === chat.id ? 1 : 0,
              }}
            >
              <X size={8} strokeWidth={2.5} />
            </button>
          </div>
        );
      })}
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
