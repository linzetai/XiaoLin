import { memo, useCallback, useRef } from "react";
import { X } from "@phosphor-icons/react";
import { useTranslation } from "react-i18next";
import type { OpenFile } from "../../lib/stores/file-viewer-store";

export interface FileTabBarProps {
  openFiles: Record<string, OpenFile>;
  activeFilePath: string | null;
  onSelect: (path: string) => void;
  onClose: (path: string) => void;
}

const tabStyle: React.CSSProperties = {
  display: "inline-flex",
  alignItems: "center",
  gap: 4,
  padding: "4px 8px",
  fontSize: 11,
  fontFamily: "var(--font-mono)",
  borderRadius: 5,
  border: "none",
  cursor: "pointer",
  maxWidth: 140,
  flexShrink: 0,
  transition: "background 0.1s, color 0.1s",
};

const closeBtnStyle: React.CSSProperties = {
  display: "inline-flex",
  alignItems: "center",
  justifyContent: "center",
  width: 16,
  height: 16,
  borderRadius: 3,
  border: "none",
  background: "transparent",
  color: "inherit",
  cursor: "pointer",
  opacity: 0.6,
  flexShrink: 0,
  padding: 0,
};

interface FileTabItemProps {
  path: string;
  active: boolean;
  onSelect: (path: string) => void;
  onClose: (path: string) => void;
}

const FileTabItem = memo(function FileTabItem({
  path,
  active,
  onSelect,
  onClose,
}: FileTabItemProps) {
  const { t } = useTranslation("fileViewer");
  const name = path.split("/").pop() ?? path;

  const handleClose = useCallback(
    (e: React.MouseEvent) => {
      e.stopPropagation();
      onClose(path);
    },
    [onClose, path],
  );

  return (
    <div
      role="tab"
      tabIndex={active ? 0 : -1}
      aria-selected={active}
      style={{
        ...tabStyle,
        color: active ? "var(--fill-primary)" : "var(--fill-quaternary)",
        background: active ? "var(--bg-hover)" : "transparent",
      }}
      title={path}
      onClick={() => onSelect(path)}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          onSelect(path);
        }
      }}
      onMouseEnter={(e) => {
        if (!active) e.currentTarget.style.color = "var(--fill-secondary)";
      }}
      onMouseLeave={(e) => {
        if (!active) e.currentTarget.style.color = "var(--fill-quaternary)";
      }}
    >
      <span
        style={{
          overflow: "hidden",
          textOverflow: "ellipsis",
          whiteSpace: "nowrap",
        }}
      >
        {name}
      </span>
      <button
        type="button"
        style={closeBtnStyle}
        title={t("closeFile", { name })}
        aria-label={t("closeFile", { name })}
        onClick={handleClose}
        onMouseEnter={(e) => {
          e.currentTarget.style.opacity = "1";
          e.currentTarget.style.background = "var(--bg-tertiary)";
        }}
        onMouseLeave={(e) => {
          e.currentTarget.style.opacity = "0.6";
          e.currentTarget.style.background = "transparent";
        }}
      >
        <X size={10} weight="bold" />
      </button>
    </div>
  );
});

export const FileTabBar = memo(function FileTabBar({
  openFiles,
  activeFilePath,
  onSelect,
  onClose,
}: FileTabBarProps) {
  const { t } = useTranslation("fileViewer");
  const paths = Object.keys(openFiles);
  const containerRef = useRef<HTMLDivElement>(null);
  if (paths.length === 0) return null;

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key !== "ArrowLeft" && e.key !== "ArrowRight") return;
    const idx = activeFilePath ? paths.indexOf(activeFilePath) : -1;
    const next = e.key === "ArrowRight"
      ? paths[(idx + 1) % paths.length]
      : paths[(idx - 1 + paths.length) % paths.length];
    if (next) {
      onSelect(next);
      const el = containerRef.current?.querySelector(`[aria-selected="true"]`) as HTMLElement | null;
      requestAnimationFrame(() => el?.focus());
    }
  };

  return (
    <div
      ref={containerRef}
      role="tablist"
      aria-label={t("openFilesAriaLabel")}
      onKeyDown={handleKeyDown}
      style={{
        display: "flex",
        alignItems: "center",
        gap: 2,
        padding: "4px 6px",
        borderBottom: "1px solid var(--border-primary)",
        background: "var(--bg-primary)",
        overflowX: "auto",
        flexShrink: 0,
      }}
    >
      {paths.map((path) => (
        <FileTabItem
          key={path}
          path={path}
          active={path === activeFilePath}
          onSelect={onSelect}
          onClose={onClose}
        />
      ))}
    </div>
  );
});
