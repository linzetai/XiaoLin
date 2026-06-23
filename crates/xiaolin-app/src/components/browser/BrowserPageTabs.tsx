import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Plus, X, Globe, CircleNotch } from "@phosphor-icons/react";
import {
  useBrowserStore,
  MAX_BROWSER_PAGES,
  NEW_TAB_URL,
} from "../../lib/stores/browser-store";

function FaviconIcon({ url }: { url?: string }) {
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    setFailed(false);
  }, [url]);

  if (!url || failed) return <Globe size={14} style={{ flexShrink: 0 }} />;

  return (
    <img
      src={url}
      alt=""
      width={14}
      height={14}
      style={{ borderRadius: 2, flexShrink: 0 }}
      onError={() => setFailed(true)}
    />
  );
}

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

interface BrowserPageTabItemProps {
  pageId: string;
  isActive: boolean;
  onSelect: (pageId: string) => void;
  onClose: (e: React.MouseEvent, pageId: string) => void;
  onMiddleClick: (e: React.MouseEvent, pageId: string) => void;
  onKeyDown: (e: React.KeyboardEvent, pageId: string) => void;
}

function BrowserPageTabItem({
  pageId,
  isActive,
  onSelect,
  onClose,
  onMiddleClick,
  onKeyDown,
}: BrowserPageTabItemProps) {
  const { t } = useTranslation("browser");
  const page = useBrowserStore((s) => s.pages[pageId]);

  if (!page) return null;

  const label = page.title || page.url || t("newTab");
  const isLoading = page.loadState.state === "loading";

  return (
    <div
      role="tab"
      tabIndex={0}
      aria-selected={isActive}
      style={{
        ...tabStyle,
        background: isActive ? "var(--bg-hover)" : "transparent",
        color: isActive ? "var(--fill-primary)" : "var(--fill-quaternary)",
      }}
      onClick={() => onSelect(pageId)}
      onKeyDown={(e) => onKeyDown(e, pageId)}
      onMouseDown={(e) => onMiddleClick(e, pageId)}
      title={label}
    >
      {page.agentControlled && <span aria-hidden>🤖</span>}
      <FaviconIcon url={page.faviconUrl} />
      <span
        style={{
          overflow: "hidden",
          textOverflow: "ellipsis",
          whiteSpace: "nowrap",
        }}
      >
        {isLoading && (
          <CircleNotch
            size={10}
            className="animate-spin"
            style={{
              display: "inline-block",
              marginRight: 4,
              verticalAlign: "middle",
              color: "var(--tint)",
            }}
          />
        )}
        {label}
      </span>
      <button
        type="button"
        tabIndex={-1}
        aria-label={t("closeTab")}
        onClick={(e) => onClose(e, pageId)}
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
}

interface BrowserPageTabsProps {
  onLimitReached?: () => void;
}

export function BrowserPageTabs({ onLimitReached }: BrowserPageTabsProps) {
  const { t } = useTranslation("browser");
  const pages = useBrowserStore((s) => s.pages);
  const activePageId = useBrowserStore((s) => s.activePageId);
  const setActivePageId = useBrowserStore((s) => s.setActivePageId);
  const closePage = useBrowserStore((s) => s.closePage);
  const openPage = useBrowserStore((s) => s.openPage);
  const [localLimitToast, setLocalLimitToast] = useState(false);

  const pageIds = useMemo(() => Object.keys(pages), [pages]);
  const atLimit = pageIds.length >= MAX_BROWSER_PAGES;

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

  const handleSelect = useCallback(
    (pageId: string) => {
      void setActivePageId(pageId);
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
        {pageIds.map((pageId) => (
          <BrowserPageTabItem
            key={pageId}
            pageId={pageId}
            isActive={pageId === activePageId}
            onSelect={handleSelect}
            onClose={handleClose}
            onMiddleClick={handleMiddleClick}
            onKeyDown={handleTabKeyDown}
          />
        ))}
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
          title={atLimit ? t("maxTabs", { max: MAX_BROWSER_PAGES }) : t("newTab")}
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
          {t("tabLimitReached", { max: MAX_BROWSER_PAGES })}
        </div>
      )}
    </div>
  );
}
