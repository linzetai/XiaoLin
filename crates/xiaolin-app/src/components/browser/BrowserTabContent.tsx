import { useRef, useEffect, useCallback } from "react";
import { useBrowserStore, shouldShowBrowserWebView } from "../../lib/stores/browser-store";
import { useWorkspaceTabs } from "../shell/workspace-tabs";
import { BrowserAddressBar, type BrowserAddressBarHandle } from "./BrowserAddressBar";
import { BrowserPageTabs } from "./BrowserPageTabs";
import { BrowserPlaceholder } from "./BrowserPlaceholder";
import { DownloadNotificationBar } from "./DownloadNotificationBar";

const NEW_TAB_URL = "https://example.com";

export function BrowserPanelBody() {
  const activePageId = useBrowserStore((s) => s.activePageId);
  const layoutMode = useBrowserStore((s) => s.layoutMode);
  const panelOpen = useWorkspaceTabs((s) => s.panelOpen);
  const activeTabId = useWorkspaceTabs((s) => s.activeTabId);
  const openPage = useBrowserStore((s) => s.openPage);
  const closePage = useBrowserStore((s) => s.closePage);
  const addressBarRef = useRef<BrowserAddressBarHandle>(null);

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

  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        height: "100%",
        minHeight: 0,
      }}
    >
      <BrowserAddressBar ref={addressBarRef} pageId={activePageId} />
      <BrowserPageTabs />
      <BrowserPlaceholder pageId={activePageId} webviewVisible={webviewVisible} />
      <DownloadNotificationBar />
    </div>
  );
}

export function BrowserTabContent() {
  return <BrowserPanelBody />;
}
