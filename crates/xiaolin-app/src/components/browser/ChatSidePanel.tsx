import { useRef, useCallback, type ReactNode } from "react";
import { ChatCircle, CaretLeft } from "@phosphor-icons/react";
import {
  useBrowserStore,
  COLLAPSED_CHAT_PANEL_WIDTH,
} from "../../lib/stores/browser-store";
import { useChatMetaStore } from "../../lib/stores/chat-meta-store";

interface ChatSidePanelProps {
  children: ReactNode;
}

const resizeHandleStyle: React.CSSProperties = {
  position: "absolute",
  top: 0,
  right: 0,
  width: 4,
  height: "100%",
  cursor: "col-resize",
  zIndex: 10,
  background: "transparent",
};

export function ChatSidePanel({ children }: ChatSidePanelProps) {
  const chatPanelWidth = useBrowserStore((s) => s.chatPanelWidth);
  const chatPanelCollapsed = useBrowserStore((s) => s.chatPanelCollapsed);
  const setChatPanelWidth = useBrowserStore((s) => s.setChatPanelWidth);
  const toggleChatPanel = useBrowserStore((s) => s.toggleChatPanel);
  const unread = useChatMetaStore((s) => s.unread);

  const dragging = useRef(false);

  const handleResizeStart = useCallback(
    (e: React.MouseEvent) => {
      e.preventDefault();
      dragging.current = true;
      const startX = e.clientX;
      const startWidth = chatPanelWidth;

      const onMove = (ev: MouseEvent) => {
        if (!dragging.current) return;
        const delta = ev.clientX - startX;
        setChatPanelWidth(startWidth + delta);
      };

      const onUp = () => {
        dragging.current = false;
        document.removeEventListener("mousemove", onMove);
        document.removeEventListener("mouseup", onUp);
        document.body.style.cursor = "";
        document.body.style.userSelect = "";
      };

      document.addEventListener("mousemove", onMove);
      document.addEventListener("mouseup", onUp);
      document.body.style.cursor = "col-resize";
      document.body.style.userSelect = "none";
    },
    [chatPanelWidth, setChatPanelWidth],
  );

  const width = chatPanelCollapsed ? COLLAPSED_CHAT_PANEL_WIDTH : chatPanelWidth;

  if (chatPanelCollapsed) {
    return (
      <div
        style={{
          width: COLLAPSED_CHAT_PANEL_WIDTH,
          minWidth: COLLAPSED_CHAT_PANEL_WIDTH,
          flexShrink: 0,
          display: "flex",
          flexDirection: "column",
          alignItems: "center",
          borderRight: "1px solid var(--border-shell-subtle)",
          background: "var(--bg-card)",
          transition: "width 0.4s ease",
          cursor: "pointer",
        }}
        onClick={toggleChatPanel}
        title="Expand chat"
      >
        <div
          style={{
            padding: "12px 0",
            display: "flex",
            flexDirection: "column",
            alignItems: "center",
            gap: 8,
            position: "relative",
          }}
        >
          <ChatCircle size={20} style={{ color: "var(--fill-secondary)" }} />
          {unread > 0 && (
            <span
              style={{
                position: "absolute",
                top: 8,
                right: 4,
                fontSize: 9,
                minWidth: 14,
                height: 14,
                lineHeight: "14px",
                textAlign: "center",
                borderRadius: 7,
                background: "var(--tint)",
                color: "#fff",
                padding: "0 3px",
                animation: unread > 0 ? "browser-pulse 2s ease infinite" : undefined,
              }}
            >
              {unread > 99 ? "99+" : unread}
            </span>
          )}
        </div>
        <style>{`
          @keyframes browser-pulse {
            0%, 100% { opacity: 1; }
            50% { opacity: 0.6; }
          }
        `}</style>
      </div>
    );
  }

  return (
    <div
      style={{
        position: "relative",
        width,
        minWidth: width,
        flexShrink: 0,
        display: "flex",
        flexDirection: "column",
        borderRight: "1px solid var(--border-shell-subtle)",
        background: "var(--bg-card)",
        transition: "width 0.4s ease",
        minHeight: 0,
      }}
    >
      <div
        style={{
          display: "flex",
          alignItems: "center",
          justifyContent: "space-between",
          padding: "6px 8px",
          borderBottom: "1px solid var(--border-shell-subtle)",
          flexShrink: 0,
        }}
      >
        <span style={{ fontSize: 12, fontWeight: 500, color: "var(--fill-secondary)" }}>Chat</span>
        <button
          type="button"
          onClick={toggleChatPanel}
          title="Collapse chat"
          style={{
            width: 24,
            height: 24,
            border: "none",
            borderRadius: 5,
            background: "transparent",
            cursor: "pointer",
            display: "flex",
            alignItems: "center",
            justifyContent: "center",
            color: "var(--fill-quaternary)",
          }}
        >
          <CaretLeft size={14} />
        </button>
      </div>
      <div style={{ flex: 1, minHeight: 0, overflow: "hidden" }}>{children}</div>
      <div
        style={resizeHandleStyle}
        onMouseDown={handleResizeStart}
        onMouseEnter={(e) => {
          e.currentTarget.style.background = "var(--tint)";
          e.currentTarget.style.opacity = "0.35";
        }}
        onMouseLeave={(e) => {
          if (!dragging.current) {
            e.currentTarget.style.background = "transparent";
            e.currentTarget.style.opacity = "1";
          }
        }}
      />
    </div>
  );
}
