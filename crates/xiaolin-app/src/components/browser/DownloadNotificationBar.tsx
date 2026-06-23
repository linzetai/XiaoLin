import { useCallback } from "react";
import { useTranslation } from "react-i18next";
import { X, FolderOpen, FileArrowDown } from "@phosphor-icons/react";
import { open } from "@tauri-apps/plugin-shell";
import { useBrowserStore } from "../../lib/stores/browser-store";

export function DownloadNotificationBar() {
  const { t } = useTranslation("browser");
  const downloads = useBrowserStore((s) => s.downloads);
  const dismissDownload = useBrowserStore((s) => s.dismissDownload);

  const handleOpenFile = useCallback(async (path: string) => {
    try {
      await open(path);
    } catch (e) {
      console.warn("[browser] failed to open file:", e);
    }
  }, []);

  const handleOpenFolder = useCallback(async (path: string) => {
    const dir = path.replace(/[/\\][^/\\]+$/, "");
    try {
      await open(dir || path);
    } catch (e) {
      console.warn("[browser] failed to open folder:", e);
    }
  }, []);

  if (downloads.length === 0) return null;

  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        gap: 4,
        padding: "6px 8px",
        borderTop: "1px solid var(--border-shell-subtle)",
        background: "var(--bg-secondary)",
        flexShrink: 0,
      }}
    >
      {downloads.map((dl) => (
        <div
          key={dl.id}
          style={{
            display: "flex",
            alignItems: "center",
            gap: 8,
            padding: "4px 6px",
            borderRadius: 6,
            background: "var(--bg-hover)",
            fontSize: 12,
          }}
        >
          <FileArrowDown size={14} style={{ flexShrink: 0, color: "var(--fill-secondary)" }} />
          <div style={{ flex: 1, minWidth: 0 }}>
            <div
              style={{
                overflow: "hidden",
                textOverflow: "ellipsis",
                whiteSpace: "nowrap",
                color: "var(--fill-primary)",
              }}
            >
              {dl.filename}
            </div>
            <div style={{ fontSize: 10, color: "var(--fill-quaternary)" }}>
              {dl.status === "downloading" && t("downloading")}
              {dl.status === "finished" && t("downloadComplete")}
              {dl.status === "failed" && t("downloadFailed")}
            </div>
          </div>
          {dl.status === "finished" && dl.path && (
            <>
              <button
                type="button"
                onClick={() => void handleOpenFile(dl.path!)}
                style={actionBtnStyle}
              >
                {t("openFile")}
              </button>
              <button
                type="button"
                onClick={() => void handleOpenFolder(dl.path!)}
                style={actionBtnStyle}
              >
                <FolderOpen size={12} />
                {t("openFolder")}
              </button>
            </>
          )}
          <button
            type="button"
            onClick={() => dismissDownload(dl.id)}
            style={{
              ...actionBtnStyle,
              padding: 2,
              color: "var(--fill-quaternary)",
            }}
            title={t("dismiss")}
          >
            <X size={12} />
          </button>
        </div>
      ))}
    </div>
  );
}

const actionBtnStyle: React.CSSProperties = {
  fontSize: 11,
  padding: "2px 6px",
  borderRadius: 4,
  border: "none",
  background: "var(--bg-card)",
  color: "var(--fill-secondary)",
  cursor: "pointer",
  display: "inline-flex",
  alignItems: "center",
  gap: 4,
  flexShrink: 0,
};
