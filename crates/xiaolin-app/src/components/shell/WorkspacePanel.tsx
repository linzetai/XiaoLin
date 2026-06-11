import { useRef, useCallback, type CSSProperties } from "react";
import { SidebarSimple } from "@phosphor-icons/react";
import { useTranslation } from "react-i18next";
import { useWorkspaceTabs } from "./workspace-tabs";

const tabBtnStyle: CSSProperties = {
  padding: "4px 8px",
  fontSize: 12,
  fontWeight: 500,
  borderRadius: 5,
  cursor: "pointer",
  border: "none",
  background: "transparent",
  transition: "all 0.1s",
};

const iconBtnStyle: CSSProperties = {
  width: 24,
  height: 24,
  borderRadius: 5,
  border: "none",
  background: "transparent",
  cursor: "pointer",
  display: "flex",
  alignItems: "center",
  justifyContent: "center",
  color: "var(--fill-quaternary)",
  transition: "background 0.12s",
};

const resizeHandleStyle: CSSProperties = {
  position: "absolute",
  top: 0,
  left: 0,
  width: 4,
  height: "100%",
  cursor: "col-resize",
  zIndex: 10,
  background: "transparent",
  transition: "background 0.15s",
};

export function WorkspacePanel() {
  const { t } = useTranslation("sidebar");
  const tabs = useWorkspaceTabs((s) => s.tabs);
  const activeTabId = useWorkspaceTabs((s) => s.activeTabId);
  const setActiveTab = useWorkspaceTabs((s) => s.setActiveTab);
  const panelOpen = useWorkspaceTabs((s) => s.panelOpen);
  const panelWidth = useWorkspaceTabs((s) => s.panelWidth);
  const setPanelWidth = useWorkspaceTabs((s) => s.setPanelWidth);
  const togglePanel = useWorkspaceTabs((s) => s.togglePanel);

  const resizeRef = useRef<HTMLDivElement>(null);
  const dragging = useRef(false);

  const handleResizeStart = useCallback(
    (e: React.MouseEvent) => {
      e.preventDefault();
      dragging.current = true;
      const startX = e.clientX;
      const startWidth = panelWidth;

      const onMove = (ev: MouseEvent) => {
        if (!dragging.current) return;
        const delta = startX - ev.clientX;
        setPanelWidth(startWidth + delta);
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
    [panelWidth, setPanelWidth]
  );

  if (!panelOpen || tabs.length === 0) return null;

  const activeTab = tabs.find((t) => t.id === activeTabId) ?? tabs[0];
  const ActiveComponent = activeTab?.component;
  const FooterComponent = activeTab?.footerComponent;

  return (
    <div
      className="workspace-panel"
      style={{
        position: "relative",
        width: panelWidth,
        minWidth: panelWidth,
        flexShrink: 0,
        display: "flex",
        flexDirection: "column",
        borderLeft: "1px solid var(--border-shell-subtle)",
        minHeight: 0,
      }}
    >
      {/* Resize handle */}
      <div
        ref={resizeRef}
        style={resizeHandleStyle}
        onMouseDown={handleResizeStart}
        onMouseEnter={(e) => { e.currentTarget.style.background = "var(--fill-accent, var(--tint, #58a6ff))"; e.currentTarget.style.opacity = "0.4"; }}
        onMouseLeave={(e) => { if (!dragging.current) { e.currentTarget.style.background = "transparent"; e.currentTarget.style.opacity = "1"; } }}
      />

      {/* Tab bar */}
      <div
        style={{
          display: "flex",
          alignItems: "center",
          padding: "7px 10px 5px",
          gap: 2,
          borderBottom: "1px solid var(--border-shell-subtle)",
        }}
      >
        {tabs.map((tab) => {
          const Icon = tab.icon;
          const active = tab.id === (activeTabId ?? tabs[0]?.id);
          return (
            <button
              key={tab.id}
              type="button"
              style={{
                ...tabBtnStyle,
                color: active ? "var(--fill-primary)" : "var(--fill-quaternary)",
                background: active ? "var(--bg-hover)" : "transparent",
              }}
              onClick={() => setActiveTab(tab.id)}
              onMouseEnter={(e) => { if (!active) e.currentTarget.style.color = "var(--fill-secondary)"; }}
              onMouseLeave={(e) => { if (!active) e.currentTarget.style.color = "var(--fill-quaternary)"; }}
            >
              <span style={{ display: "inline-flex", alignItems: "center", gap: 4 }}>
                <Icon />
                {tab.label}
                {tab.badge != null && tab.badge !== false && (
                  <span
                    style={{
                      fontSize: 10,
                      background: "var(--tint)",
                      color: "#fff",
                      borderRadius: 8,
                      padding: "0 5px",
                      minWidth: 16,
                      textAlign: "center",
                      lineHeight: "16px",
                    }}
                  >
                    {tab.badge === true ? "" : tab.badge}
                  </span>
                )}
              </span>
            </button>
          );
        })}

        <div style={{ flex: 1 }} />

        <button
          type="button"
          style={iconBtnStyle}
          title={t("closePanel")}
          onClick={togglePanel}
          onMouseEnter={(e) => { e.currentTarget.style.background = "var(--bg-hover)"; }}
          onMouseLeave={(e) => { e.currentTarget.style.background = "transparent"; }}
        >
          <SidebarSimple size={13} />
        </button>
      </div>

      {/* Body */}
      <div style={{ flex: 1, overflowY: "auto", minHeight: 0 }}>
        {ActiveComponent && <ActiveComponent />}
      </div>

      {/* Footer */}
      {FooterComponent && (
        <div style={{ borderTop: "1px solid var(--border-shell-subtle)" }}>
          <FooterComponent />
        </div>
      )}
    </div>
  );
}
