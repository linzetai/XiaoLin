import { useEffect } from "react";
import { useTranslation } from "react-i18next";
import { useChatMetaStore } from "../../lib/stores/chat-meta-store";
import { useFileViewerStore } from "../../lib/stores/file-viewer-store";
import { useWorkspaceTabs } from "./workspace-tabs";

export interface OpenFileEventDetail {
  path: string;
  line?: number;
  workDir?: string;
  source?: string;
}

/**
 * Workspace tab adapter for the built-in file viewer.
 * Phase 7 will add the full split-pane layout; this stub wires event listeners.
 */
export function FilesTabContent() {
  const { t } = useTranslation("sidebar");
  const workDir = useChatMetaStore((s) => s.chats[s.activeChatId]?.workDir ?? null);
  const openFile = useFileViewerStore((s) => s.openFile);
  const openFiles = useFileViewerStore((s) => s.openFiles);
  const activeFilePath = useFileViewerStore((s) => s.activeFilePath);
  const artifacts = useFileViewerStore((s) => s.artifacts);

  useEffect(() => {
    const handler = (e: Event) => {
      const detail = (e as CustomEvent<OpenFileEventDetail>).detail;
      if (!detail?.path) return;

      const chatId = useChatMetaStore.getState().activeChatId;
      const chat = useChatMetaStore.getState().chats[chatId];
      const resolvedWorkDir = detail.workDir ?? chat?.workDir ?? "";
      if (!resolvedWorkDir) return;

      void openFile(detail.path, resolvedWorkDir, detail.line);
      useWorkspaceTabs.getState().setActiveTab("files");
    };

    window.addEventListener("xiaolin:open-file", handler);
    return () => window.removeEventListener("xiaolin:open-file", handler);
  }, [openFile]);

  const openCount = Object.keys(openFiles).length;
  const activeFile = activeFilePath ? openFiles[activeFilePath] : null;

  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        height: "100%",
        padding: 16,
        color: "var(--fill-secondary)",
        fontSize: 13,
        gap: 8,
      }}
    >
      <div style={{ fontWeight: 600, color: "var(--fill-primary)" }}>Files</div>
      {!workDir && (
        <p style={{ margin: 0, opacity: 0.7 }}>{t("setWorkDir", { ns: "chat", defaultValue: "Set a working directory to browse files." })}</p>
      )}
      {workDir && openCount === 0 && artifacts.length === 0 && (
        <p style={{ margin: 0, opacity: 0.7 }}>
          {t("filesEmptyHint", { defaultValue: "Open a file from chat or the file tree." })}
        </p>
      )}
      {openCount > 0 && (
        <div>
          <div style={{ opacity: 0.7, marginBottom: 4 }}>
            {t("filesOpenCount", { defaultValue: "{{count}} open", count: openCount })}
          </div>
          {activeFile && (
            <div style={{ fontFamily: "var(--font-mono, monospace)", fontSize: 12 }}>
              {activeFile.path.split("/").pop()}
            </div>
          )}
        </div>
      )}
      {artifacts.length > 0 && (
        <div style={{ opacity: 0.7 }}>
          {t("filesArtifactCount", { defaultValue: "{{count}} artifacts", count: artifacts.length })}
        </div>
      )}
    </div>
  );
}
