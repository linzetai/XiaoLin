import { useRef, useEffect, useCallback, useState } from "react";
import { Globe } from "@phosphor-icons/react";
import { useBrowserStore, shouldShowBrowserWebView, hasBrowserPages } from "../../lib/stores/browser-store";
import { useWorkspaceTabs } from "../shell/workspace-tabs";
import { BrowserAddressBar, type BrowserAddressBarHandle } from "./BrowserAddressBar";
import { BrowserPageTabs } from "./BrowserPageTabs";
import { BrowserPlaceholder } from "./BrowserPlaceholder";
import { DownloadNotificationBar } from "./DownloadNotificationBar";
import { BrowserNetworkSettings } from "./BrowserNetworkSettings";
import { AgentOperationLog } from "./AgentOperationLog";

const NEW_TAB_URL = "https://example.com";

function BrowserEmptyState() {
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
      <span style={{ fontSize: 13 }}>内置浏览器</span>
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
        打开新页面
      </button>
      <span style={{ fontSize: 11, color: "var(--fill-quaternary)", marginTop: 2 }}>
        Ctrl+T 快速打开
      </span>
    </div>
  );
}

export function BrowserPanelBody() {
  const activePageId = useBrowserStore((s) => s.activePageId);
  const layoutMode = useBrowserStore((s) => s.layoutMode);
  const panelOpen = useWorkspaceTabs((s) => s.panelOpen);
  const activeTabId = useWorkspaceTabs((s) => s.activeTabId);
  const hasPages = useBrowserStore((s) => Object.keys(s.pages).length > 0);
  const openPage = useBrowserStore((s) => s.openPage);
  const closePage = useBrowserStore((s) => s.closePage);
  const addressBarRef = useRef<BrowserAddressBarHandle>(null);
  const [networkSettingsOpen, setNetworkSettingsOpen] = useState(false);

  const webviewVisible = shouldShowBrowserWebView({ layoutMode, panelOpen, activeTabId });

  const handleNewTab = useCallback(async () => {
    await openPage(NEW_TAB_URL);
    requestAnimationFrame(() => {
      addressBarRef.current?.focus();
      addressBarRef.current?.selectAll();
    });
  }, [openPage]);

  useEffect(() => {
    function onKeyDown(e: KeyboardEvent) {
      const mod = e.ctrlKey || e.metaKey;
      if (!mod) return;

      if (e.key === "t" || e.key === "T") {
        e.preventDefault();
        void handleNewTab();
        return;
      }
      if (e.key === "w" || e.key === "W") {
        e.preventDefault();
        if (activePageId) void closePage(activePageId);
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
  }, [activePageId, closePage, handleNewTab]);

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
      <BrowserAddressBar
        ref={addressBarRef}
        pageId={activePageId}
        onOpenNetworkSettings={() => setNetworkSettingsOpen(true)}
      />
      <BrowserPageTabs />
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
