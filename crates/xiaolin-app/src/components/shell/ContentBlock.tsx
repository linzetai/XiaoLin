import type { ReactNode } from "react";
import { WorkspacePanel } from "./WorkspacePanel";
import { useWorkspaceTabs } from "./workspace-tabs";

export function ContentBlock({ children }: { children: ReactNode }) {
  const panelOpen = useWorkspaceTabs((s) => s.panelOpen);
  const hasTabs = useWorkspaceTabs((s) => s.tabs.length > 0);
  const showPanel = panelOpen && hasTabs;

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
