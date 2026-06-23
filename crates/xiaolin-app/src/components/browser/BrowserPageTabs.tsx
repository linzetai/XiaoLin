import { useCallback, useState } from "react";
import { Plus, X, Globe } from "@phosphor-icons/react";
import {
  useBrowserStore,
  MAX_BROWSER_PAGES,
} from "../../lib/stores/browser-store";

const NEW_TAB_URL = "https://example.com";

const tabStyle: React.CSSProperties = {
  display: "inline-flex",
  alignItems: "center",
  gap: 4,
  padding: "4px 8px",
  borderRadius: 6,
  cursor: "pointer",
  fontSize: 11,
  maxWidth: 140,
  flexShrink: 0,
  transition: "background 0.1s",
};

interface BrowserPageTabsProps {
  onLimitReached?: () => void;
}

export function BrowserPageTabs({ onLimitReached }: BrowserPageTabsProps) {
  const pages = useBrowserStore((s) => s.pages);
  const activePageId = useBrowserStore((s) => s.activePageId);
  const setActivePageId = useBrowserStore((s) => s.setActivePageId);
  const closePage = useBrowserStore((s) => s.closePage);
  const openPage = useBrowserStore((s) => s.openPage);
  const [localLimitToast, setLocalLimitToast] = useState(false);

  const pageList = Object.values(pages);
  const atLimit = pageList.length >= MAX_BROWSER_PAGES;

  const showLimitToast = useCallback(() => {
    if (onLimitReached) {
      onLimitReached();
      return;
    }
    setLocalLimitToast(true);
    window.setTimeout(() => setLocalLimitToast(false), 2500);
  }, [onLimitReached]);

  const handleNewTab = useCallback(() => {
    if (atLimit) {
      console.warn(`[browser] maximum ${MAX_BROWSER_PAGES} tabs reached`);
      showLimitToast();
      return;
    }
    void openPage(NEW_TAB_URL);
  }, [atLimit, openPage, showLimitToast]);

  const handleClose = useCallback(
    (e: React.MouseEvent, pageId: string) => {
      e.stopPropagation();
      void closePage(pageId);
    },
    [closePage],
  );

  const handleMiddleClick = useCallback(
    (e: React.MouseEvent, pageId: string) => {
      if (e.button === 1) {
        e.preventDefault();
        void closePage(pageId);
      }
    },
    [closePage],
  );

  const handleTabKeyDown = useCallback(
    (e: React.KeyboardEvent, pageId: string) => {
      if (e.key === "Enter" || e.key === " ") {
        e.preventDefault();
        void setActivePageId(pageId);
      }
    },
    [setActivePageId],
  );

  return (
    <div style={{ flexShrink: 0 }}>
      <div
        role="tablist"
        style={{
          display: "flex",
          alignItems: "center",
          gap: 2,
          padding: "4px 8px",
          borderBottom: "1px solid var(--border-shell-subtle)",
          overflowX: "auto",
        }}
      >
        {pageList.map((page) => {
          const active = page.pageId === activePageId;
          const label = page.title || page.url || "New Tab";
          return (
            <div
              key={page.pageId}
              role="tab"
              tabIndex={0}
              aria-selected={active}
              style={{
                ...tabStyle,
                background: active ? "var(--bg-hover)" : "transparent",
                color: active ? "var(--fill-primary)" : "var(--fill-quaternary)",
              }}
              onClick={() => void setActivePageId(page.pageId)}
              onKeyDown={(e) => handleTabKeyDown(e, page.pageId)}
              onMouseDown={(e) => handleMiddleClick(e, page.pageId)}
              title={label}
            >
              {page.agentControlled && <span aria-hidden>🤖</span>}
              <Globe size={12} style={{ flexShrink: 0 }} />
              <span
                style={{
                  overflow: "hidden",
                  textOverflow: "ellipsis",
                  whiteSpace: "nowrap",
                }}
              >
                {page.loadState.state === "loading" && (
                  <span
                    style={{
                      display: "inline-block",
                      width: 8,
                      height: 8,
                      marginRight: 4,
                      borderRadius: "50%",
                      border: "1.5px solid var(--fill-quaternary)",
                      borderTopColor: "var(--tint)",
                      animation: "browser-spin 0.8s linear infinite",
                      verticalAlign: "middle",
                    }}
                  />
                )}
                {label}
              </span>
              <button
                type="button"
                aria-label="关闭标签"
                onClick={(e) => handleClose(e, page.pageId)}
                style={{
                  display: "flex",
                  padding: 2,
                  borderRadius: 3,
                  border: "none",
                  background: "transparent",
                  cursor: "pointer",
                  color: "var(--fill-quaternary)",
                }}
              >
                <X size={10} />
              </button>
            </div>
          );
        })}
        <button
          type="button"
          style={{
            ...tabStyle,
            width: 28,
            padding: 4,
            justifyContent: "center",
            border: "none",
            color: atLimit ? "var(--fill-quaternary)" : "var(--fill-tertiary)",
            opacity: atLimit ? 0.5 : 1,
          }}
          title={atLimit ? `Maximum ${MAX_BROWSER_PAGES} tabs` : "New tab"}
          disabled={atLimit}
          onClick={handleNewTab}
        >
          <Plus size={14} />
        </button>
      </div>
      {!onLimitReached && localLimitToast && (
        <div
          style={{
            padding: "6px 12px",
            fontSize: 12,
            color: "var(--fill-secondary)",
            background: "var(--bg-hover)",
            borderBottom: "1px solid var(--border-shell-subtle)",
            textAlign: "center",
          }}
        >
          已达标签页上限（{MAX_BROWSER_PAGES} 个）
        </div>
      )}
    </div>
  );
}
