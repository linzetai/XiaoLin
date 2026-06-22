import { memo, useCallback, useRef, useState } from "react";
import {
  Copy,
  ArrowSquareOut,
  TextAlignLeft,
  Code,
  Eye,
} from "@phosphor-icons/react";
import { useTranslation } from "react-i18next";
import { open } from "@tauri-apps/plugin-shell";
import { isImagePath, isSvgPath } from "../../lib/file-utils";
import type { OpenFile } from "../../lib/stores/file-viewer-store";

export interface FileToolbarProps {
  file: OpenFile;
  wordWrap: boolean;
  onWordWrapChange: (wrap: boolean) => void;
  onViewModeChange: (mode: "code" | "preview") => void;
}

const iconBtnStyle: React.CSSProperties = {
  display: "inline-flex",
  alignItems: "center",
  justifyContent: "center",
  width: 26,
  height: 26,
  borderRadius: 5,
  border: "none",
  background: "transparent",
  color: "var(--fill-tertiary)",
  cursor: "pointer",
  flexShrink: 0,
  transition: "background 0.1s, color 0.1s",
};

function supportsViewModeToggle(path: string, language: string): boolean {
  if (isImagePath(path)) return isSvgPath(path);
  return language === "markdown";
}

export const FileToolbar = memo(function FileToolbar({
  file,
  wordWrap,
  onWordWrapChange,
  onViewModeChange,
}: FileToolbarProps) {
  const { t } = useTranslation("sidebar");
  const [copied, setCopied] = useState(false);
  const copyTimerRef = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);

  const fileName = file.path.split("/").pop() ?? file.path;
  const showViewToggle = supportsViewModeToggle(file.path, file.language);
  const isPreview = file.viewMode === "preview";

  const handleCopy = useCallback(async () => {
    try {
      await navigator.clipboard.writeText(file.content);
      setCopied(true);
      clearTimeout(copyTimerRef.current);
      copyTimerRef.current = setTimeout(() => setCopied(false), 1500);
    } catch {
      /* clipboard may be unavailable */
    }
  }, [file.content]);

  const handleOpenExternal = useCallback(async () => {
    try {
      await open(file.path);
    } catch (err) {
      console.warn("[FileToolbar] failed to open externally:", file.path, err);
    }
  }, [file.path]);

  return (
    <div
      style={{
        display: "flex",
        alignItems: "center",
        gap: 6,
        padding: "5px 8px",
        borderBottom: "1px solid var(--border-primary)",
        background: "var(--bg-secondary)",
        flexShrink: 0,
        minHeight: 32,
      }}
    >
      <span
        title={file.path}
        style={{
          flex: 1,
          minWidth: 0,
          fontSize: 12,
          fontFamily: "var(--font-mono)",
          color: "var(--fill-primary)",
          overflow: "hidden",
          textOverflow: "ellipsis",
          whiteSpace: "nowrap",
        }}
      >
        {fileName}
      </span>

      <button
        type="button"
        style={iconBtnStyle}
        title={copied ? t("copied", { defaultValue: "Copied" }) : t("copyContent", { defaultValue: "Copy content" })}
        onClick={() => void handleCopy()}
        onMouseEnter={(e) => {
          e.currentTarget.style.background = "var(--bg-hover)";
          e.currentTarget.style.color = "var(--fill-secondary)";
        }}
        onMouseLeave={(e) => {
          e.currentTarget.style.background = "transparent";
          e.currentTarget.style.color = "var(--fill-tertiary)";
        }}
      >
        <Copy size={14} />
      </button>

      <button
        type="button"
        style={iconBtnStyle}
        title={t("openExternal", { defaultValue: "Open in external app" })}
        onClick={() => void handleOpenExternal()}
        onMouseEnter={(e) => {
          e.currentTarget.style.background = "var(--bg-hover)";
          e.currentTarget.style.color = "var(--fill-secondary)";
        }}
        onMouseLeave={(e) => {
          e.currentTarget.style.background = "transparent";
          e.currentTarget.style.color = "var(--fill-tertiary)";
        }}
      >
        <ArrowSquareOut size={14} />
      </button>

      {!isImagePath(file.path) && (
        <button
          type="button"
          style={{
            ...iconBtnStyle,
            color: wordWrap ? "var(--fill-primary)" : "var(--fill-tertiary)",
            background: wordWrap ? "var(--bg-hover)" : "transparent",
          }}
          title={t("wordWrap", { defaultValue: "Toggle word wrap" })}
          onClick={() => onWordWrapChange(!wordWrap)}
          onMouseEnter={(e) => {
            if (!wordWrap) {
              e.currentTarget.style.background = "var(--bg-hover)";
              e.currentTarget.style.color = "var(--fill-secondary)";
            }
          }}
          onMouseLeave={(e) => {
            if (!wordWrap) {
              e.currentTarget.style.background = "transparent";
              e.currentTarget.style.color = "var(--fill-tertiary)";
            }
          }}
        >
          <TextAlignLeft size={14} />
        </button>
      )}

      {showViewToggle && (
        <button
          type="button"
          style={{
            ...iconBtnStyle,
            color: "var(--fill-tertiary)",
          }}
          title={
            isPreview
              ? t("viewSource", { defaultValue: "View source" })
              : t("viewPreview", { defaultValue: "View preview" })
          }
          onClick={() => onViewModeChange(isPreview ? "code" : "preview")}
          onMouseEnter={(e) => {
            e.currentTarget.style.background = "var(--bg-hover)";
            e.currentTarget.style.color = "var(--fill-secondary)";
          }}
          onMouseLeave={(e) => {
            e.currentTarget.style.background = "transparent";
            e.currentTarget.style.color = "var(--fill-tertiary)";
          }}
        >
          {isPreview ? <Code size={14} /> : <Eye size={14} />}
        </button>
      )}
    </div>
  );
});
