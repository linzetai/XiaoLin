import { useState, useEffect, useCallback, type CSSProperties, type ReactNode, type MouseEvent as RME } from "react";
import { useTranslation } from "react-i18next";
import {
  PanelLeft,
  PanelBottom,
  Sun,
  Moon,
  Square,
  Minus,
  Maximize2,
  X,
} from "lucide-react";
import { useUIStore, useGitStore } from "../../lib/stores";
import { useThemeStore } from "../../lib/theme";
import { useWorkspaceTabs } from "./workspace-tabs";

const isTauri =
  typeof window !== "undefined" &&
  ("__TAURI_INTERNALS__" in window || "__TAURI__" in window);

async function onDragMouseDown(e: RME) {
  if (!isTauri || e.button !== 0) return;
  const { getCurrentWindow } = await import("@tauri-apps/api/window");
  getCurrentWindow().startDragging();
}

async function onDragDoubleClick() {
  if (!isTauri) return;
  const { getCurrentWindow } = await import("@tauri-apps/api/window");
  getCurrentWindow().toggleMaximize();
}

const iconBtnBase: CSSProperties = {
  width: 28,
  height: 28,
  borderRadius: 6,
  border: "none",
  background: "transparent",
  color: "var(--fill-quaternary)",
  cursor: "pointer",
  display: "flex",
  alignItems: "center",
  justifyContent: "center",
  transition: "background 0.12s, color 0.12s",
};

const ICON_SIZE = 15;

function IconButton({
  children,
  title,
  onClick,
  style,
  active,
}: {
  children: ReactNode;
  title?: string;
  onClick?: ((e: RME<HTMLButtonElement>) => void) | (() => void);
  style?: CSSProperties;
  active?: boolean;
}) {
  return (
    <button
      type="button"
      style={{
        ...iconBtnBase,
        ...(active ? { background: "var(--bg-hover)", color: "var(--fill-secondary)" } : {}),
        ...style,
      }}
      title={title}
      onClick={onClick}
      onMouseEnter={(e) => { e.currentTarget.style.background = "var(--bg-hover)"; }}
      onMouseLeave={(e) => {
        e.currentTarget.style.background = active ? "var(--bg-hover)" : "transparent";
      }}
    >
      {children}
    </button>
  );
}

function WindowControls() {
  const { t } = useTranslation("common");
  const [isMaximized, setIsMaximized] = useState(false);

  useEffect(() => {
    if (!isTauri) return;
    let cancelled = false;
    let unlistenFn: (() => void) | undefined;
    (async () => {
      const { getCurrentWindow } = await import("@tauri-apps/api/window");
      const win = getCurrentWindow();
      if (!cancelled) setIsMaximized(await win.isMaximized());
      unlistenFn = await win.onResized(async () => {
        if (!cancelled) setIsMaximized(await win.isMaximized());
      });
    })();
    return () => { cancelled = true; unlistenFn?.(); };
  }, []);

  const minimize = useCallback(async () => {
    const { getCurrentWindow } = await import("@tauri-apps/api/window");
    getCurrentWindow().minimize();
  }, []);

  const toggleMaximize = useCallback(async () => {
    const { getCurrentWindow } = await import("@tauri-apps/api/window");
    getCurrentWindow().toggleMaximize();
  }, []);

  const close = useCallback(async () => {
    const { getCurrentWindow } = await import("@tauri-apps/api/window");
    getCurrentWindow().close();
  }, []);

  if (!isTauri) return null;

  const wc: CSSProperties = {
    ...iconBtnBase,
    width: 36,
    borderRadius: 0,
  };

  return (
    <div style={{ display: "flex", alignItems: "stretch", height: "100%", marginLeft: 4 }}>
      <div style={{ width: 1, alignSelf: "center", height: 14, background: "var(--separator)" }} />
      <button type="button" style={wc} onClick={minimize} title={t("minimize")}
        onMouseEnter={(e) => { e.currentTarget.style.background = "var(--bg-hover)"; }}
        onMouseLeave={(e) => { e.currentTarget.style.background = "transparent"; }}>
        <Minus size={14} strokeWidth={1.2} />
      </button>
      <button type="button" style={wc} onClick={toggleMaximize}
        title={isMaximized ? t("restore") : t("maximize")}
        onMouseEnter={(e) => { e.currentTarget.style.background = "var(--bg-hover)"; }}
        onMouseLeave={(e) => { e.currentTarget.style.background = "transparent"; }}>
        {isMaximized ? <Maximize2 size={14} strokeWidth={1.2} /> : <Square size={14} strokeWidth={1.2} />}
      </button>
      <button type="button" style={{ ...wc, borderRadius: "0 0 0 0" }} onClick={close} title={t("close")}
        onMouseEnter={(e) => { e.currentTarget.style.background = "#E81123"; e.currentTarget.style.color = "#fff"; }}
        onMouseLeave={(e) => { e.currentTarget.style.background = "transparent"; e.currentTarget.style.color = "var(--fill-quaternary)"; }}>
        <X size={14} strokeWidth={1.2} />
      </button>
    </div>
  );
}

function PanelLayoutToggle({ panelOpen, togglePanel }: { panelOpen: boolean; togglePanel: () => void }) {
  const gitStatus = useGitStore((s) => s.status);
  const changeCount = (gitStatus?.staged?.length ?? 0) + (gitStatus?.unstaged?.length ?? 0) + (gitStatus?.untracked?.length ?? 0);
  const showDot = !panelOpen && gitStatus?.isGitRepo && changeCount > 0;

  return (
    <div style={{ display: "flex", gap: 1, position: "relative" }}>
      <IconButton title="单栏" onClick={() => { if (panelOpen) togglePanel(); }} active={!panelOpen}>
        <svg viewBox="0 0 24 24" width={14} height={14} fill="none" stroke="currentColor" strokeWidth={1.7}>
          <rect x="3" y="3" width="18" height="18" rx="3" />
        </svg>
      </IconButton>
      <IconButton title="分栏" onClick={() => { if (!panelOpen) togglePanel(); }} active={panelOpen}>
        <svg viewBox="0 0 24 24" width={14} height={14} fill="none" stroke="currentColor" strokeWidth={1.7}>
          <rect x="3" y="3" width="18" height="18" rx="3" />
          <line x1="12" y1="3" x2="12" y2="21" />
        </svg>
      </IconButton>
      {showDot && (
        <span style={{
          position: "absolute", top: 2, right: 2,
          width: 7, height: 7, borderRadius: "50%",
          background: "var(--tint)",
          border: "1.5px solid var(--bg-shell)",
          pointerEvents: "none",
        }} />
      )}
    </div>
  );
}

export function AppHeader() {
  const { t } = useTranslation("header");
  const sidebarCollapsed = useUIStore((s) => s.sidebarCollapsed);
  const toggleSidebar = useUIStore((s) => s.toggleSidebar);
  const resolved = useThemeStore((s) => s.resolved);
  const setMode = useThemeStore((s) => s.setMode);

  const togglePanel = useWorkspaceTabs((s) => s.togglePanel);
  const panelOpen = useWorkspaceTabs((s) => s.panelOpen);

  const handleThemeToggle = useCallback(() => {
    setMode(resolved === "light" ? "dark" : "light");
  }, [resolved, setMode]);

  return (
    <header
      className="app-header"
      style={{
        height: "var(--header-h)",
        minHeight: "var(--header-h)",
        display: "flex",
        alignItems: "center",
        flexShrink: 0,
        background: "var(--bg-shell)",
        padding: "0 12px",
        position: "relative",
        zIndex: 10,
      } as CSSProperties}
    >
      {/* Left: nav tools */}
      <div style={{ display: "flex", alignItems: "center", gap: 2 }}>
        <IconButton title={t("toggleSidebar")} onClick={toggleSidebar} active={!sidebarCollapsed}>
          <PanelLeft size={16} strokeWidth={1.7} />
        </IconButton>
        <IconButton title={t("togglePanel")} onClick={togglePanel} active={panelOpen}>
          <PanelBottom size={16} strokeWidth={1.7} />
        </IconButton>
      </div>

      {/* Center: drag region */}
      <div
        data-tauri-drag-region=""
        onMouseDown={onDragMouseDown}
        onDoubleClick={onDragDoubleClick}
        style={{ flex: 1, minWidth: 0 }}
      />

      {/* Right: actions + window controls */}
      <div style={{ display: "flex", alignItems: "center", gap: 4 }}>
        <IconButton title={resolved === "light" ? t("darkMode") : t("lightMode")} onClick={handleThemeToggle}>
          {resolved === "light" ? <Sun size={ICON_SIZE} strokeWidth={1.7} /> : <Moon size={ICON_SIZE} strokeWidth={1.7} />}
        </IconButton>
        <PanelLayoutToggle panelOpen={panelOpen} togglePanel={togglePanel} />
        <WindowControls />
      </div>
    </header>
  );
}
