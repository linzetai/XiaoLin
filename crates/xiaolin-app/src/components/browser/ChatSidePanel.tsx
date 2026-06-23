import { useRef, useCallback, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { CaretRight } from "@phosphor-icons/react";
import {
  useBrowserStore,
  COLLAPSED_CHAT_PANEL_WIDTH,
  MIN_CHAT_PANEL_WIDTH,
} from "../../lib/stores/browser-store";

interface ChatSidePanelProps {
  children: ReactNode;
}

const resizeHandleStyle: React.CSSProperties = {
  position: "absolute",
  top: 0,
  left: 0,
  width: 4,
  height: "100%",
  cursor: "col-resize",
  zIndex: 10,
  background: "transparent",
};

export function ChatSidePanel({ children }: ChatSidePanelProps) {
  const { t } = useTranslation("browser");
  const chatPanelWidth = useBrowserStore((s) => s.chatPanelWidth);
  const chatPanelCollapsed = useBrowserStore((s) => s.chatPanelCollapsed);
  const setChatPanelWidth = useBrowserStore((s) => s.setChatPanelWidth);
  const toggleChatPanel = useBrowserStore((s) => s.toggleChatPanel);

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
        setChatPanelWidth(startWidth - delta);
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

  return (
    <div
      style={{
        position: "relative",
        width,
        minWidth: chatPanelCollapsed ? 0 : MIN_CHAT_PANEL_WIDTH,
        flexShrink: 0,
        display: "flex",
        flexDirection: "column",
        borderLeft: chatPanelCollapsed ? "none" : "1px solid var(--border-shell-subtle)",
        background: "var(--bg-card)",
        transition: "width 200ms ease",
        minHeight: 0,
        overflow: chatPanelCollapsed ? "hidden" : undefined,
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
        <span style={{ fontSize: 12, fontWeight: 500, color: "var(--fill-secondary)" }}>{t("chat")}</span>
        <button
          type="button"
          onClick={toggleChatPanel}
          title={t("collapsePanel")}
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
          <CaretRight size={14} />
        </button>
      </div>
      <div style={{ flex: 1, minHeight: 0, overflow: "hidden" }}>{children}</div>
      {!chatPanelCollapsed && (
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
      )}
    </div>
  );
}
