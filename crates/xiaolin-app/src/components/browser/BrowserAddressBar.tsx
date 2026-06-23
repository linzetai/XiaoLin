import {
  useRef,
  useState,
  useEffect,
  useCallback,
  forwardRef,
  useImperativeHandle,
  type CSSProperties,
} from "react";
import {
  ArrowLeft,
  ArrowRight,
  ArrowClockwise,
  X,
  Lock,
  LockOpen,
  ArrowsOut,
  ArrowsIn,
  Globe,
  ChatCircle,
} from "@phosphor-icons/react";
import {
  useBrowserStore,
  browserGoBack,
  browserGoForward,
  browserReload,
  browserStopLoading,
  browserRequestTakeover,
  normalizeNavUrl,
  isHttpsUrl,
} from "../../lib/stores/browser-store";
import { useChatMetaStore } from "../../lib/stores/chat-meta-store";

export interface BrowserAddressBarHandle {
  focus: () => void;
  selectAll: () => void;
}

interface BrowserAddressBarProps {
  pageId: string | null;
  onOpenNetworkSettings?: () => void;
}

const iconBtnStyle: CSSProperties = {
  width: 28,
  height: 28,
  borderRadius: 6,
  border: "none",
  background: "transparent",
  cursor: "pointer",
  display: "flex",
  alignItems: "center",
  justifyContent: "center",
  color: "var(--fill-tertiary)",
  flexShrink: 0,
  transition: "background 0.1s",
};

export const BrowserAddressBar = forwardRef<BrowserAddressBarHandle, BrowserAddressBarProps>(
  function BrowserAddressBar({ pageId, onOpenNetworkSettings }, ref) {
    const page = useBrowserStore((s) => (pageId ? s.pages[pageId] : null));
    const layoutMode = useBrowserStore((s) => s.layoutMode);
    const setLayoutMode = useBrowserStore((s) => s.setLayoutMode);
    const chatPanelCollapsed = useBrowserStore((s) => s.chatPanelCollapsed);
    const toggleChatPanel = useBrowserStore((s) => s.toggleChatPanel);
    const navigate = useBrowserStore((s) => s.navigate);
    const unread = useChatMetaStore((s) => s.unread);

    const inputRef = useRef<HTMLInputElement>(null);
    const [editing, setEditing] = useState(false);
    const [inputValue, setInputValue] = useState("");

    useImperativeHandle(ref, () => ({
      focus: () => {
        inputRef.current?.focus();
        inputRef.current?.select();
      },
      selectAll: () => inputRef.current?.select(),
    }));

    const url = page?.url ?? "";
    const isLoading = page?.loadState.state === "loading";
    const agentControlled = page?.agentControlled ?? false;
    const secure = url ? isHttpsUrl(url) : false;

    useEffect(() => {
      if (!editing) {
        setInputValue(url);
      }
    }, [url, editing]);

    const handleSubmit = useCallback(
      (e: React.FormEvent) => {
        e.preventDefault();
        if (!pageId) return;
        const normalized = normalizeNavUrl(inputValue);
        if (!normalized) return;
        setEditing(false);
        void navigate(pageId, normalized);
      },
      [pageId, inputValue, navigate],
    );

    const handleReload = useCallback(() => {
      if (pageId) void browserReload(pageId);
    }, [pageId]);

    const handleStopLoading = useCallback(() => {
      if (pageId) void browserStopLoading(pageId);
    }, [pageId]);

    const toggleLayout = useCallback(() => {
      void setLayoutMode(layoutMode === "panel" ? "fullwidth" : "panel");
    }, [layoutMode, setLayoutMode]);

    return (
      <div style={{ display: "flex", flexDirection: "column", flexShrink: 0 }}>
        {agentControlled && (
          <div
            style={{
              display: "flex",
              alignItems: "center",
              justifyContent: "space-between",
              padding: "4px 10px",
              background: "rgba(88, 166, 255, 0.12)",
              borderBottom: "1px solid var(--border-shell-subtle)",
              fontSize: 11,
              color: "var(--fill-secondary)",
            }}
          >
            <span>Agent 操作中</span>
            <button
              type="button"
              onClick={() => pageId && void browserRequestTakeover(pageId)}
              style={{
                padding: "2px 8px",
                borderRadius: 4,
                border: "none",
                background: "var(--bg-card)",
                cursor: "pointer",
                fontSize: 11,
                color: "var(--fill-primary)",
              }}
            >
              取回控制
            </button>
          </div>
        )}
        <form
          onSubmit={handleSubmit}
          style={{
            display: "flex",
            alignItems: "center",
            gap: 4,
            padding: "6px 8px",
            borderBottom: "1px solid var(--border-shell-subtle)",
          }}
        >
          <button
            type="button"
            style={iconBtnStyle}
            title="Back"
            disabled={!pageId}
            onClick={() => pageId && void browserGoBack(pageId)}
          >
            <ArrowLeft size={14} />
          </button>
          <button
            type="button"
            style={iconBtnStyle}
            title="Forward"
            disabled={!pageId}
            onClick={() => pageId && void browserGoForward(pageId)}
          >
            <ArrowRight size={14} />
          </button>
          {isLoading ? (
            <button
              type="button"
              style={iconBtnStyle}
              title="停止加载"
              aria-label="停止加载"
              disabled={!pageId}
              onClick={handleStopLoading}
            >
              <X size={16} />
            </button>
          ) : (
            <button
              type="button"
              style={iconBtnStyle}
              title="重新加载"
              aria-label="重新加载"
              disabled={!pageId}
              onClick={handleReload}
            >
              <ArrowClockwise size={16} />
            </button>
          )}

          <div
            style={{
              flex: 1,
              display: "flex",
              alignItems: "center",
              gap: 6,
              padding: "4px 8px",
              borderRadius: 6,
              background: "var(--bg-hover)",
              minWidth: 0,
            }}
          >
            {url && (
              secure ? (
                <Lock size={12} style={{ flexShrink: 0, color: "var(--fill-tertiary)" }} />
              ) : (
                <LockOpen size={12} style={{ flexShrink: 0, color: "var(--fill-quaternary)" }} />
              )
            )}
            <input
              ref={inputRef}
              type="text"
              value={inputValue}
              onChange={(e) => { setInputValue(e.target.value); setEditing(true); }}
              onFocus={() => setEditing(true)}
              onBlur={() => setEditing(false)}
              placeholder="Enter URL…"
              disabled={!pageId}
              style={{
                flex: 1,
                minWidth: 0,
                border: "none",
                background: "transparent",
                outline: "none",
                fontSize: 12,
                color: "var(--fill-primary)",
              }}
            />
          </div>

          <button
            type="button"
            style={iconBtnStyle}
            title="Network settings"
            onClick={onOpenNetworkSettings}
          >
            <Globe size={14} />
          </button>

          {layoutMode === "fullwidth" && (
            <button
              type="button"
              style={{
                ...iconBtnStyle,
                position: "relative",
                ...(chatPanelCollapsed ? {} : { background: "var(--bg-tertiary)", color: "var(--fill-primary)" }),
              }}
              title="Chat 面板"
              aria-label={chatPanelCollapsed ? "展开 Chat" : "折叠 Chat"}
              aria-expanded={!chatPanelCollapsed}
              onClick={toggleChatPanel}
            >
              <ChatCircle size={14} />
              {unread > 0 && (
                <span
                  style={{
                    position: "absolute",
                    top: 2,
                    right: 2,
                    minWidth: unread > 9 ? 16 : 8,
                    height: unread > 9 ? 14 : 8,
                    padding: unread > 9 ? "0 3px" : 0,
                    borderRadius: unread > 9 ? 7 : "50%",
                    background: "var(--red, #FF3B30)",
                    color: "#fff",
                    fontSize: 9,
                    fontWeight: 600,
                    lineHeight: "14px",
                    textAlign: "center",
                    pointerEvents: "none",
                    animation: "pulse-subtle 1.5s ease-in-out infinite",
                  }}
                >
                  {unread > 9 ? "9+" : unread > 1 ? unread : null}
                </span>
              )}
            </button>
          )}

          <button
            type="button"
            style={iconBtnStyle}
            title={layoutMode === "panel" ? "Full width" : "Side panel"}
            onClick={toggleLayout}
          >
            {layoutMode === "panel" ? <ArrowsOut size={14} /> : <ArrowsIn size={14} />}
          </button>
        </form>
      </div>
    );
  },
);
