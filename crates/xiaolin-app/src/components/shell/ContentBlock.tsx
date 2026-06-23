import type { ReactNode } from "react";
import { WorkspacePanel } from "./WorkspacePanel";
import { useWorkspaceTabs } from "./workspace-tabs";
import { useBrowserStore } from "../../lib/stores/browser-store";
import { ChatSidePanel } from "../browser/ChatSidePanel";
import { BrowserFullPanel } from "../browser/BrowserFullPanel";

export function ContentBlock({ children }: { children: ReactNode }) {
  const panelOpen = useWorkspaceTabs((s) => s.panelOpen);
  const hasTabs = useWorkspaceTabs((s) => s.tabs.length > 0);
  const showPanel = panelOpen && hasTabs;
  const layoutMode = useBrowserStore((s) => s.layoutMode);
  const hasBrowserPages = useBrowserStore((s) => Object.keys(s.pages).length > 0);
  const layoutTransitioning = useBrowserStore((s) => s.layoutTransitioning);
  const fullwidthBrowser = layoutMode === "fullwidth" && hasBrowserPages;

  if (fullwidthBrowser) {
    return (
      <div
        className="content-block content-block--fullwidth-browser"
        style={{
          flex: 1,
          minWidth: 0,
          display: "flex",
          flexDirection: "row",
          background: "var(--bg-card)",
          borderRadius: showPanel
            ? "var(--card-r) 0 0 var(--card-r)"
            : "var(--card-r)",
          margin: "0 0 var(--gap-shell) 0",
          overflow: "hidden",
          opacity: layoutTransitioning ? 0.92 : 1,
          transition: "opacity 0.4s ease",
        }}
      >
        <ChatSidePanel>{children}</ChatSidePanel>
        <BrowserFullPanel />
        {showPanel && <WorkspacePanel />}
      </div>
    );
  }

  return (
    <div
      className="content-block"
      style={{
        flex: 1,
        minWidth: 0,
        display: "flex",
        flexDirection: "row",
        background: "var(--bg-card)",
        borderRadius: showPanel
          ? "var(--card-r) 0 0 var(--card-r)"
          : "var(--card-r)",
        margin: "0 0 var(--gap-shell) 0",
        overflow: "hidden",
      }}
    >
      <div style={{ flex: 1, minWidth: 0, display: "flex", flexDirection: "column" }}>
        {children}
      </div>
      {showPanel && <WorkspacePanel />}
    </div>
  );
}
