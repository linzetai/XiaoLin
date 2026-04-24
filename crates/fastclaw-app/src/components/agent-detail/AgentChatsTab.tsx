import { useState } from "react";
import { X, Search, FolderOpen } from "lucide-react";
import { useAgentStore, type Chat } from "../../lib/agent-store";
import { ListContainer } from "./common";

function ChatRow({ chat, isActive, onClick, onClose, isLast }: {
  chat: Chat;
  isActive: boolean;
  onClick: () => void;
  onClose?: () => void;
  isLast: boolean;
}) {
  return (
    <div
      className="group relative flex w-full cursor-pointer flex-col gap-1 px-3 py-2.5 text-left transition-colors duration-100 hover:bg-[var(--bg-hover)]"
      style={{
        background: isActive ? "var(--tint-bg)" : "transparent",
        borderBottom: isLast ? "none" : "0.5px solid var(--separator)",
      }}
      onClick={onClick}
    >
      {onClose && (
        <button
          onClick={(e) => { e.stopPropagation(); onClose(); }}
          className="absolute top-2 right-2 flex h-5 w-5 items-center justify-center rounded-md opacity-0 transition-opacity duration-100 group-hover:opacity-100"
          style={{ color: "var(--fill-quaternary)" }}
          title="关闭会话"
        >
          <X size={8} strokeWidth={2.5} />
        </button>
      )}
      <div className="flex items-start justify-between gap-2">
        <span className="min-w-0 flex-1 truncate text-[13px] font-medium leading-tight" style={{ color: "var(--fill-primary)" }} title={chat.title}>
          {chat.title}
        </span>
        {isActive && (
          <span className="mt-0.5 shrink-0 rounded-full px-1.5 py-0.5 text-[10px] font-medium" style={{ background: "var(--fill-primary)", color: "var(--fill-inverse)" }}>当前</span>
        )}
      </div>
      {chat.workDir && (
        <div className="flex items-center gap-1.5 text-[10px] font-mono" style={{ color: "var(--fill-quaternary)" }}>
          <FolderOpen size={10} strokeWidth={1.5} />
          <span className="truncate">{chat.workDir.replace(/^\/home\/[^/]+\//, "~/")}</span>
        </div>
      )}
      <div className="flex items-center gap-2 text-[11px]" style={{ color: "var(--fill-tertiary)" }}>
        <span>{chat.createdAt.toLocaleDateString("zh-CN", { month: "numeric", day: "numeric", hour: "2-digit", minute: "2-digit" })}</span>
        <span>·</span>
        <span>{chat.messageCount} 条消息</span>
      </div>
    </div>
  );
}

/* ━━━ Chats Tab ━━━ */

export function ChatsTab() {
  const activeAgentId = useAgentStore((s) => s.activeAgentId);
  const agentChats = useAgentStore((s) => s.agentChats);
  const setActiveChat = useAgentStore((s) => s.setActiveChat);
  const reopenChat = useAgentStore((s) => s.reopenChat);
  const closeChat = useAgentStore((s) => s.closeChat);
  const ac = agentChats[activeAgentId];

  const [chatQuery, setChatQuery] = useState("");

  if (!ac) return null;

  const openChats = ac.chatList.filter((c) => c.open);
  const closedChats = ac.chatList.filter((c) => !c.open);

  const filteredOpen = chatQuery
    ? openChats.filter((c) => c.title.toLowerCase().includes(chatQuery.toLowerCase()))
    : openChats;
  const filteredClosed = chatQuery
    ? closedChats.filter((c) => c.title.toLowerCase().includes(chatQuery.toLowerCase()))
    : closedChats;

  return (
    <div className="p-4">
      {/* Search */}
      <div
        className="mb-4 flex items-center gap-2.5 rounded-[10px] px-3 py-[7px]"
        style={{ background: "var(--bg-hover)" }}
      >
        <Search size={12} strokeWidth={1.5} style={{ color: "var(--fill-tertiary)" }} />
        <input
          type="text"
          value={chatQuery}
          onChange={(e) => setChatQuery(e.target.value)}
          placeholder="搜索会话..."
          className="min-w-0 flex-1 bg-transparent text-[12px] outline-none"
          style={{ color: "var(--fill-primary)" }}
        />
        {chatQuery && (
          <button onClick={() => setChatQuery("")} className="cursor-pointer" style={{ color: "var(--fill-quaternary)" }}>
            <X size={10} strokeWidth={2} />
          </button>
        )}
      </div>

      {filteredOpen.length > 0 && (
        <div className="mb-4">
          <div className="mb-1.5 flex items-center gap-1.5 px-1">
            <span className="text-[11px] font-medium uppercase tracking-wider" style={{ color: "var(--fill-tertiary)" }}>已打开</span>
            <span className="text-[10px]" style={{ color: "var(--fill-quaternary)" }}>({filteredOpen.length})</span>
          </div>
          <ListContainer>
            {filteredOpen.map((chat, i) => (
              <ChatRow
                key={chat.id}
                chat={chat}
                isActive={chat.id === ac.activeChatId}
                onClick={() => setActiveChat(activeAgentId, chat.id)}
                onClose={() => closeChat(activeAgentId, chat.id)}
                isLast={i === filteredOpen.length - 1}
              />
            ))}
          </ListContainer>
        </div>
      )}

      {filteredClosed.length > 0 && (
        <div>
          <div className="mb-1.5 flex items-center gap-1.5 px-1">
            <span className="text-[11px] font-medium uppercase tracking-wider" style={{ color: "var(--fill-tertiary)" }}>历史会话</span>
            <span className="text-[10px]" style={{ color: "var(--fill-quaternary)" }}>({filteredClosed.length})</span>
          </div>
          <ListContainer>
            {filteredClosed.map((chat, i) => (
              <ChatRow
                key={chat.id}
                chat={chat}
                isActive={false}
                onClick={() => reopenChat(activeAgentId, chat.id)}
                isLast={i === filteredClosed.length - 1}
              />
            ))}
          </ListContainer>
        </div>
      )}

      {ac.chatList.length === 0 && (
        <div className="py-12 text-center">
          <p className="text-[13px]" style={{ color: "var(--fill-tertiary)" }}>暂无会话</p>
        </div>
      )}
    </div>
  );
}
