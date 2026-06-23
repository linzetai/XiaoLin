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
  Lock,
  LockOpen,
  ArrowsOut,
  ArrowsIn,
} from "@phosphor-icons/react";
import {
  useBrowserStore,
  browserGoBack,
  browserGoForward,
  browserReload,
  normalizeNavUrl,
  isHttpsUrl,
} from "../../lib/stores/browser-store";

export interface BrowserAddressBarHandle {
  focus: () => void;
  selectAll: () => void;
}

interface BrowserAddressBarProps {
  pageId: string | null;
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
  function BrowserAddressBar({ pageId }, ref) {
    const page = useBrowserStore((s) => (pageId ? s.pages[pageId] : null));
    const layoutMode = useBrowserStore((s) => s.layoutMode);
    const setLayoutMode = useBrowserStore((s) => s.setLayoutMode);
    const navigate = useBrowserStore((s) => s.navigate);
    const setAgentControlled = useBrowserStore((s) => s.setAgentControlled);

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
      [pageId, navigate],
    );

    const handleReload = useCallback(() => {
      if (pageId) void browserReload(pageId);
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
              onClick={() => pageId && setAgentControlled(pageId, false)}
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
          <button
            type="button"
            style={{
              ...iconBtnStyle,
              animation: isLoading ? "browser-spin 1s linear infinite" : undefined,
            }}
            title={isLoading ? "Loading" : "Reload"}
            disabled={!pageId}
            onClick={handleReload}
          >
            <ArrowClockwise size={14} />
          </button>

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
            title={layoutMode === "panel" ? "Full width" : "Side panel"}
            onClick={toggleLayout}
          >
            {layoutMode === "panel" ? <ArrowsOut size={14} /> : <ArrowsIn size={14} />}
          </button>
        </form>
        <style>{`
          @keyframes browser-spin {
            from { transform: rotate(0deg); }
            to { transform: rotate(360deg); }
          }
        `}</style>
      </div>
    );
  },
);
