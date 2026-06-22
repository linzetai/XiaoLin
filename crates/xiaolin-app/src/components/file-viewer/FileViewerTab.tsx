import { useCallback, useEffect, useState } from "react";
import { File as FileIcon, FolderOpen } from "@phosphor-icons/react";
import { useTranslation } from "react-i18next";
import { useChatMetaStore } from "../../lib/stores/chat-meta-store";
import { useFileViewerStore } from "../../lib/stores/file-viewer-store";
import { useWorkspaceTabs } from "../shell/workspace-tabs";
import { isTauri } from "../../lib/transport";
import { isImagePath } from "../../lib/file-utils";
import { FileListSidebar } from "./FileListSidebar";
import { FileTabBar } from "./FileTabBar";
import { FileToolbar } from "./FileToolbar";
import { FileViewer } from "./FileViewer";

const NARROW_PANEL_THRESHOLD = 400;
const FIRST_OPEN_TARGET_WIDTH = 500;

let _hasExpandedPanel = false;
const MIN_PANEL_WIDTH = 260;
const MAX_PANEL_WIDTH = 700;

async function tryExpandPanelWidth(targetWidth: number): Promise<void> {
  const { panelWidth, setPanelWidth } = useWorkspaceTabs.getState();
  if (panelWidth >= targetWidth) return;

  const clampedTarget = Math.max(MIN_PANEL_WIDTH, Math.min(MAX_PANEL_WIDTH, targetWidth));
  const delta = clampedTarget - panelWidth;
  if (delta <= 0) return;

  if (!isTauri) {
    setPanelWidth(clampedTarget);
    return;
  }

  try {
    const { getCurrentWindow, currentMonitor } = await import("@tauri-apps/api/window");
    const { LogicalSize } = await import("@tauri-apps/api/dpi");
    const win = getCurrentWindow();

    if (await win.isMaximized()) {
      setPanelWidth(clampedTarget);
      return;
    }

    const size = await win.innerSize();
    const pos = await win.outerPosition();
    const monitor = await currentMonitor();
    const scale = await win.scaleFactor();
    const logicalSize = size.toLogical(scale);

    if (monitor) {
      const availableRight = monitor.position.x + monitor.size.width;
      const windowRight = pos.x + size.width + delta;
      if (windowRight > availableRight) return;
    }

    await win.setSize(new LogicalSize(logicalSize.width + delta, logicalSize.height));
    setPanelWidth(clampedTarget);
  } catch {
    /* ignore resize failures */
  }
}

function FileEmptyState({
  workDir,
  onBrowse,
}: {
  workDir: string | null;
  onBrowse: () => void;
}) {
  const { t } = useTranslation("sidebar");

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
        color: "var(--fill-tertiary)",
        minHeight: 0,
      }}
    >
      <FileIcon size={40} weight="thin" style={{ opacity: 0.4 }} />
      <p style={{ margin: 0, fontSize: 13, color: "var(--fill-secondary)" }}>
        {t("filesEmpty", { defaultValue: "No files yet" })}
      </p>
      {!workDir && (
        <p style={{ margin: 0, fontSize: 12, opacity: 0.7, textAlign: "center" }}>
          {t("setWorkDir", { ns: "chat", defaultValue: "Set a working directory to browse files." })}
        </p>
      )}
      {workDir && (
        <button
          type="button"
          style={{
            display: "inline-flex",
            alignItems: "center",
            gap: 6,
            padding: "6px 12px",
            fontSize: 12,
            borderRadius: 6,
            border: "1px solid var(--border-primary)",
            background: "var(--bg-secondary)",
            color: "var(--fill-primary)",
            cursor: "pointer",
          }}
          onClick={onBrowse}
          onMouseEnter={(e) => {
            e.currentTarget.style.background = "var(--bg-hover)";
          }}
          onMouseLeave={(e) => {
            e.currentTarget.style.background = "var(--bg-secondary)";
          }}
        >
          <FolderOpen size={14} />
          {t("browseWorkDir", { defaultValue: "Browse working directory" })}
        </button>
      )}
    </div>
  );
}

export function FileViewerTab() {
  const { t } = useTranslation("sidebar");
  const workDir = useChatMetaStore((s) => s.chats[s.activeChatId]?.workDir ?? null);
  const panelWidth = useWorkspaceTabs((s) => s.panelWidth);

  const openFiles = useFileViewerStore((s) => s.openFiles);
  const activeFilePath = useFileViewerStore((s) => s.activeFilePath);
  const artifacts = useFileViewerStore((s) => s.artifacts);
  const fileListCollapsed = useFileViewerStore((s) => s.fileListCollapsed);
  const lastOpenError = useFileViewerStore((s) => s.lastOpenError);
  const staleFiles = useFileViewerStore((s) => s.staleFiles);
  const openFile = useFileViewerStore((s) => s.openFile);
  const closeFile = useFileViewerStore((s) => s.closeFile);
  const setActiveFile = useFileViewerStore((s) => s.setActiveFile);
  const setViewMode = useFileViewerStore((s) => s.setViewMode);
  const toggleFileList = useFileViewerStore((s) => s.toggleFileList);
  const reloadFile = useFileViewerStore((s) => s.reloadFile);
  const dismissStale = useFileViewerStore((s) => s.dismissStale);
  const clearOpenError = useFileViewerStore((s) => s.clearOpenError);

  const [wordWrap, setWordWrap] = useState(false);
  const [overlayOpen, setOverlayOpen] = useState(false);
  const [browseActive, setBrowseActive] = useState(true);

  const openCount = Object.keys(openFiles).length;
  const activeFile = activeFilePath ? openFiles[activeFilePath] : null;
  const isActiveStale = activeFilePath ? staleFiles.has(activeFilePath) : false;
  const autoCollapsed = panelWidth < NARROW_PANEL_THRESHOLD;
  const effectiveCollapsed = fileListCollapsed || autoCollapsed;
  const overlayMode = autoCollapsed;

  useEffect(() => {
    if (_hasExpandedPanel) return;
    _hasExpandedPanel = true;
    void tryExpandPanelWidth(FIRST_OPEN_TARGET_WIDTH);
  }, []);

  useEffect(() => {
    useWorkspaceTabs.getState().setTabBadge("files", undefined);
    let prevTabId = useWorkspaceTabs.getState().activeTabId;
    return useWorkspaceTabs.subscribe((state) => {
      if (state.activeTabId === "files" && prevTabId !== "files") {
        state.setTabBadge("files", undefined);
      }
      prevTabId = state.activeTabId;
    });
  }, []);

  useEffect(() => {
    if (!overlayOpen) return;
    const handler = (e: MouseEvent) => {
      const target = e.target as Element | null;
      if (!target) return;
      if (target.closest("[data-file-list-overlay]") || target.closest("[data-file-list-strip]")) {
        return;
      }
      setOverlayOpen(false);
    };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, [overlayOpen]);

  const handleOpenFile = useCallback(
    (path: string) => {
      if (!workDir) return;
      void openFile(path, workDir);
      setOverlayOpen(false);
    },
    [openFile, workDir],
  );

  const handleBrowseActivate = useCallback(() => {
    setBrowseActive(true);
    if (effectiveCollapsed && overlayMode) {
      setOverlayOpen(true);
    }
  }, [effectiveCollapsed, overlayMode]);

  const handleViewModeChange = useCallback(
    (mode: "code" | "preview") => {
      if (!activeFilePath) return;
      setViewMode(activeFilePath, mode);
    },
    [activeFilePath, setViewMode],
  );

  const showEmptyState = openCount === 0 && artifacts.length === 0;

  return (
    <div
      style={{
        display: "flex",
        flexDirection: "row",
        height: "100%",
        minHeight: 0,
        position: "relative",
        background: "var(--bg-primary)",
      }}
    >
      <FileListSidebar
        workDir={workDir}
        artifacts={artifacts}
        activeFilePath={activeFilePath}
        collapsed={effectiveCollapsed}
        overlayMode={overlayMode}
        overlayOpen={overlayOpen}
        browseActive={browseActive}
        onToggleCollapse={toggleFileList}
        onOpenOverlay={() => setOverlayOpen(true)}
        onOpenFile={handleOpenFile}
        onBrowseActivate={handleBrowseActivate}
      />

      <div
        style={{
          flex: 1,
          display: "flex",
          flexDirection: "column",
          minWidth: 0,
          minHeight: 0,
        }}
      >
        {openCount > 0 && (
          <FileTabBar
            openFiles={openFiles}
            activeFilePath={activeFilePath}
            onSelect={setActiveFile}
            onClose={closeFile}
          />
        )}

        {lastOpenError && (
          <div
            style={{
              padding: "6px 10px",
              fontSize: 11,
              color: "var(--red-text)",
              background: "var(--bg-secondary)",
              borderBottom: "1px solid var(--border-primary)",
              display: "flex",
              alignItems: "center",
              justifyContent: "space-between",
              flexShrink: 0,
            }}
          >
            <span>{lastOpenError}</span>
            <button
              type="button"
              style={{
                border: "none",
                background: "transparent",
                color: "inherit",
                cursor: "pointer",
                fontSize: 11,
              }}
              onClick={clearOpenError}
            >
              {t("dismiss", { defaultValue: "Dismiss" })}
            </button>
          </div>
        )}

        {activeFile && workDir ? (
          <>
            {!isImagePath(activeFile.path) && (
              <FileToolbar
                file={activeFile}
                wordWrap={wordWrap}
                onWordWrapChange={setWordWrap}
                onViewModeChange={handleViewModeChange}
              />
            )}
            {isActiveStale && (
              <div
                style={{
                  display: "flex",
                  alignItems: "center",
                  justifyContent: "space-between",
                  padding: "4px 10px",
                  fontSize: 11,
                  background: "var(--warning-bg, rgba(237,137,54,0.12))",
                  color: "var(--warning-text, #ed8936)",
                  borderBottom: "1px solid var(--border-primary)",
                  flexShrink: 0,
                }}
              >
                <span>{t("fileModified", { defaultValue: "This file has been modified by the agent." })}</span>
                <div style={{ display: "flex", gap: 8 }}>
                  <button
                    type="button"
                    style={{
                      background: "none",
                      border: "none",
                      color: "inherit",
                      cursor: "pointer",
                      fontSize: 11,
                      textDecoration: "underline",
                    }}
                    onClick={() => {
                      if (activeFilePath && workDir) void reloadFile(activeFilePath, workDir);
                    }}
                  >
                    {t("reloadFile", { defaultValue: "Reload" })}
                  </button>
                  <button
                    type="button"
                    style={{
                      background: "none",
                      border: "none",
                      color: "inherit",
                      cursor: "pointer",
                      fontSize: 11,
                    }}
                    onClick={() => {
                      if (activeFilePath) dismissStale(activeFilePath);
                    }}
                  >
                    {t("dismiss", { defaultValue: "Dismiss" })}
                  </button>
                </div>
              </div>
            )}
            <div style={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "column" }}>
              <FileViewer
                file={activeFile}
                workDir={workDir}
                wordWrap={wordWrap}
                onViewModeChange={handleViewModeChange}
              />
            </div>
          </>
        ) : showEmptyState ? (
          <FileEmptyState workDir={workDir} onBrowse={handleBrowseActivate} />
        ) : (
          <div
            style={{
              flex: 1,
              display: "flex",
              alignItems: "center",
              justifyContent: "center",
              fontSize: 12,
              color: "var(--fill-quaternary)",
              padding: 16,
            }}
          >
            {t("filesSelectHint", { defaultValue: "Select a file from the list" })}
          </div>
        )}
      </div>
    </div>
  );
}
