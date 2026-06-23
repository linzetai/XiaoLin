import { useRef, useEffect, useCallback, useState } from "react";
import { useTranslation } from "react-i18next";
import { Globe } from "@phosphor-icons/react";
import {
  useBrowserStore,
  shouldShowBrowserWebView,
  browserReload,
  browserStopLoading,
  MAX_BROWSER_PAGES,
} from "../../lib/stores/browser-store";
import { useWorkspaceTabs } from "../shell/workspace-tabs";
import { BrowserAddressBar, type BrowserAddressBarHandle } from "./BrowserAddressBar";
import { BrowserPageTabs } from "./BrowserPageTabs";
import { BrowserProgressBar } from "./BrowserProgressBar";
import { BrowserPlaceholder } from "./BrowserPlaceholder";
import { DownloadNotificationBar } from "./DownloadNotificationBar";
import { BrowserNetworkSettings } from "./BrowserNetworkSettings";
import { AgentOperationLog } from "./AgentOperationLog";

const NEW_TAB_URL = "https://example.com";

function BrowserEmptyState() {
  const { t } = useTranslation("browser");
  const openPage = useBrowserStore((s) => s.openPage);

  return (
    <div
      style={{
        flex: 1,
        display: "flex",
        flexDirection: "column",
        alignItems: "center",
        justifyContent: "center",
        gap: 12,
        padding: 24,
        color: "var(--fill-quaternary)",
      }}
    >
      <Globe size={40} weight="thin" />
      <span style={{ fontSize: 13 }}>{t("builtInBrowser")}</span>
      <button
        type="button"
        onClick={() => void openPage(NEW_TAB_URL)}
        style={{
          marginTop: 4,
          padding: "6px 16px",
          borderRadius: 6,
          border: "1px solid var(--border-shell-subtle)",
          background: "var(--bg-hover)",
          color: "var(--fill-secondary)",
          fontSize: 12,
          cursor: "pointer",
          transition: "background 0.15s",
        }}
      >
        {t("openNewPage")}
      </button>
      <span style={{ fontSize: 11, color: "var(--fill-quaternary)", marginTop: 2 }}>
        {t("ctrlTQuickOpen")}
      </span>
    </div>
  );
}

export function BrowserPanelBody() {
  const { t } = useTranslation("browser");
  const activePageId = useBrowserStore((s) => s.activePageId);
  const activePage = useBrowserStore((s) =>
    s.activePageId ? s.pages[s.activePageId] : null,
  );
  const layoutMode = useBrowserStore((s) => s.layoutMode);
  const panelOpen = useWorkspaceTabs((s) => s.panelOpen);
  const activeTabId = useWorkspaceTabs((s) => s.activeTabId);
  const hasPages = useBrowserStore((s) => Object.keys(s.pages).length > 0);
  const openPage = useBrowserStore((s) => s.openPage);
  const closePage = useBrowserStore((s) => s.closePage);
  const addressBarRef = useRef<BrowserAddressBarHandle>(null);
  const networkSettingsOpen = useBrowserStore((s) => s.networkSettingsOpen);
  const setNetworkSettingsOpen = useBrowserStore((s) => s.setNetworkSettingsOpen);
  const [limitToast, setLimitToast] = useState(false);

  const webviewVisible = shouldShowBrowserWebView({ layoutMode, panelOpen, activeTabId });

  const showLimitToast = useCallback(() => {
    setLimitToast(true);
    window.setTimeout(() => setLimitToast(false), 2500);
  }, []);

  const handleNewTab = useCallback(async () => {
    const pageCount = Object.keys(useBrowserStore.getState().pages).length;
    if (pageCount >= MAX_BROWSER_PAGES) {
      showLimitToast();
      return;
    }
    const pageId = await openPage(NEW_TAB_URL);
    if (!pageId) {
      showLimitToast();
      return;
    }
    requestAnimationFrame(() => {
      addressBarRef.current?.focus();
      addressBarRef.current?.selectAll();
    });
  }, [openPage, showLimitToast]);

  useEffect(() => {
    function isEditableFocused(): boolean {
      const el = document.activeElement;
      if (!el) return false;
      const tag = el.tagName;
      if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT") return true;
      if (el instanceof HTMLElement && el.isContentEditable) return true;
      return false;
    }

    function isBrowserPanelActive(): boolean {
      const ws = useWorkspaceTabs.getState();
      const { layoutMode } = useBrowserStore.getState();
      if (layoutMode === "fullwidth") return true;
      return ws.panelOpen && ws.activeTabId === "browser";
    }

    function isBrowserVisible(): boolean {
      const ws = useWorkspaceTabs.getState();
      const { layoutMode } = useBrowserStore.getState();
      return shouldShowBrowserWebView({
        layoutMode,
        panelOpen: ws.panelOpen,
        activeTabId: ws.activeTabId,
      });
    }

    function orderedPageIds(): string[] {
      return Object.values(useBrowserStore.getState().pages).map((p) => p.pageId);
    }

    function onKeyDown(e: KeyboardEvent) {
      const mod = e.ctrlKey || e.metaKey;
      const editableFocused = isEditableFocused();

      if (e.key === "F5") {
        if (editableFocused || !isBrowserVisible()) return;
        e.preventDefault();
        const pageId = useBrowserStore.getState().activePageId;
        if (pageId) void browserReload(pageId);
        return;
      }

      if (e.key === "Escape") {
        if (editableFocused || !isBrowserVisible()) return;
        const state = useBrowserStore.getState();
        const pageId = state.activePageId;
        if (!pageId) return;
        if (state.pages[pageId]?.loadState.state !== "loading") return;
        e.preventDefault();
        void browserStopLoading(pageId);
        return;
      }

      if (!isBrowserPanelActive() && !isBrowserVisible()) return;

      if (mod && e.key === "Tab") {
        if (editableFocused) return;
        if (!isBrowserVisible()) return;
        e.preventDefault();
        const ids = orderedPageIds();
        if (ids.length === 0) return;
        const { activePageId: currentId } = useBrowserStore.getState();
        const idx = currentId ? ids.indexOf(currentId) : -1;
        const nextIdx = e.shiftKey
          ? idx <= 0
            ? ids.length - 1
            : idx - 1
          : idx < 0 || idx >= ids.length - 1
            ? 0
            : idx + 1;
        void useBrowserStore.getState().setActivePageId(ids[nextIdx]!);
        return;
      }

      if (mod && !e.shiftKey && /^[1-9]$/.test(e.key)) {
        if (editableFocused) return;
        if (!isBrowserVisible()) return;
        e.preventDefault();
        const ids = orderedPageIds();
        const n = Number.parseInt(e.key, 10);
        const targetIdx = n === 9 ? ids.length - 1 : n - 1;
        if (targetIdx < 0 || targetIdx >= ids.length) return;
        void useBrowserStore.getState().setActivePageId(ids[targetIdx]!);
        return;
      }

      if (mod && !e.shiftKey && (e.key === "r" || e.key === "R")) {
        if (editableFocused || !isBrowserVisible()) return;
        e.preventDefault();
        const pageId = useBrowserStore.getState().activePageId;
        if (pageId) void browserReload(pageId);
        return;
      }

      if (editableFocused) return;
      if (!mod) return;

      if (e.key === "t" || e.key === "T") {
        e.preventDefault();
        void handleNewTab();
        return;
      }
      if (e.key === "w" || e.key === "W") {
        e.preventDefault();
        const pageId = useBrowserStore.getState().activePageId;
        if (pageId) void closePage(pageId);
        return;
      }
      if (e.key === "l" || e.key === "L") {
        e.preventDefault();
        addressBarRef.current?.focus();
        addressBarRef.current?.selectAll();
        return;
      }
      if (e.shiftKey && (e.key === "f" || e.key === "F")) {
        e.preventDefault();
        const mode = useBrowserStore.getState().layoutMode;
        void useBrowserStore.getState().setLayoutMode(mode === "panel" ? "fullwidth" : "panel");
      }
    }

    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [closePage, handleNewTab]);

  if (!hasPages) {
    return (
      <div style={{ display: "flex", flexDirection: "column", height: "100%", minHeight: 0 }}>
        <BrowserEmptyState />
      </div>
    );
  }

  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        height: "100%",
        minHeight: 0,
      }}
    >
      <BrowserPageTabs onLimitReached={showLimitToast} />
      <BrowserAddressBar
        ref={addressBarRef}
        pageId={activePageId}
        onOpenNetworkSettings={() => setNetworkSettingsOpen(true)}
      />
      <BrowserProgressBar
        loadState={activePage?.loadState.state ?? "ready"}
        resetKey={activePage?.url}
      />
      {limitToast && (
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
      <BrowserPlaceholder pageId={activePageId} webviewVisible={webviewVisible} />
      <AgentOperationLog />
      <DownloadNotificationBar />
      <BrowserNetworkSettings open={networkSettingsOpen} onClose={() => setNetworkSettingsOpen(false)} />
    </div>
  );
}

export function BrowserTabContent() {
  return <BrowserPanelBody />;
}
