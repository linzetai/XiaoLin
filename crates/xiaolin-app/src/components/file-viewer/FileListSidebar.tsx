import { memo, useCallback, useEffect, useState } from "react";
import {
  CaretLeft,
  CaretRight,
  File,
  Folder,
  FolderOpen,
} from "@phosphor-icons/react";
import { useTranslation } from "react-i18next";
import type { FileArtifact } from "../../lib/transport";
import { listDirectory, type DirEntry } from "../../lib/transport";
import { resolveFilePath } from "../../lib/stores/file-viewer-store";

const FILE_LIST_WIDTH = 180;
const FILE_LIST_COLLAPSED = 36;
const MAX_DIR_ENTRIES = 500;

const opColor: Record<FileArtifact["operation"], string> = {
  created: "var(--green-text)",
  modified: "var(--blue-text, #60a5fa)",
  deleted: "var(--red-text)",
};

const opLabel: Record<FileArtifact["operation"], string> = {
  created: "C",
  modified: "M",
  deleted: "D",
};

interface FileListSidebarProps {
  workDir: string | null;
  artifacts: FileArtifact[];
  activeFilePath: string | null;
  collapsed: boolean;
  overlayMode: boolean;
  overlayOpen: boolean;
  browseActive: boolean;
  onToggleCollapse: () => void;
  onOpenOverlay: () => void;
  onOpenFile: (path: string) => void;
  onBrowseActivate: () => void;
}

interface ArtifactRowProps {
  artifact: FileArtifact;
  active: boolean;
  onOpen: (path: string) => void;
}

const ArtifactRow = memo(function ArtifactRow({ artifact, active, onOpen }: ArtifactRowProps) {
  const name = artifact.path.split("/").pop() ?? artifact.path;
  const dir = artifact.path.includes("/")
    ? artifact.path.slice(0, artifact.path.lastIndexOf("/"))
    : "";

  if (artifact.operation === "deleted") {
    return (
      <div
        style={{
          display: "flex",
          alignItems: "center",
          padding: "4px 8px 4px 20px",
          fontSize: 11,
          fontFamily: "var(--font-mono)",
          opacity: 0.5,
        }}
      >
        <span
          style={{
            fontSize: 9,
            fontWeight: 700,
            width: 12,
            textAlign: "center",
            marginRight: 4,
            flexShrink: 0,
            color: opColor.deleted,
          }}
        >
          D
        </span>
        <span
          style={{
            flex: 1,
            overflow: "hidden",
            textOverflow: "ellipsis",
            whiteSpace: "nowrap",
            textDecoration: "line-through",
            color: "var(--fill-tertiary)",
          }}
        >
          {name}
        </span>
      </div>
    );
  }

  return (
    <div
      role="button"
      tabIndex={0}
      style={{
        display: "flex",
        alignItems: "center",
        padding: "4px 8px 4px 20px",
        fontSize: 11,
        fontFamily: "var(--font-mono)",
        cursor: "pointer",
        background: active ? "var(--bg-hover)" : "transparent",
        transition: "background 0.1s",
      }}
      onClick={() => onOpen(artifact.path)}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") onOpen(artifact.path);
      }}
      onMouseEnter={(e) => {
        if (!active) e.currentTarget.style.background = "var(--bg-hover)";
      }}
      onMouseLeave={(e) => {
        e.currentTarget.style.background = active ? "var(--bg-hover)" : "transparent";
      }}
    >
      <span
        style={{
          fontSize: 9,
          fontWeight: 700,
          width: 12,
          textAlign: "center",
          marginRight: 4,
          flexShrink: 0,
          color: opColor[artifact.operation],
        }}
      >
        {opLabel[artifact.operation]}
      </span>
      <span
        style={{
          flex: 1,
          overflow: "hidden",
          textOverflow: "ellipsis",
          whiteSpace: "nowrap",
          color: "var(--fill-primary)",
        }}
      >
        {name}
      </span>
      {dir && (
        <span
          style={{
            fontSize: 9,
            color: "var(--fill-quaternary)",
            marginLeft: 4,
            flexShrink: 0,
            maxWidth: 60,
            overflow: "hidden",
            textOverflow: "ellipsis",
            whiteSpace: "nowrap",
          }}
        >
          {dir}
        </span>
      )}
    </div>
  );
});

interface DirRowProps {
  entry: DirEntry;
  parentPath: string;
  onOpenFile: (path: string) => void;
  onNavigate: (path: string) => void;
}

const DirRow = memo(function DirRow({ entry, parentPath, onOpenFile, onNavigate }: DirRowProps) {
  const fullPath = `${parentPath.replace(/\/+$/, "")}/${entry.name}`;

  const handleClick = useCallback(() => {
    if (entry.isDir) {
      onNavigate(fullPath);
    } else {
      onOpenFile(fullPath);
    }
  }, [entry.isDir, fullPath, onNavigate, onOpenFile]);

  return (
    <div
      role="button"
      tabIndex={0}
      style={{
        display: "flex",
        alignItems: "center",
        gap: 4,
        padding: "4px 8px 4px 20px",
        fontSize: 11,
        fontFamily: "var(--font-mono)",
        cursor: "pointer",
        transition: "background 0.1s",
      }}
      onClick={handleClick}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") handleClick();
      }}
      onMouseEnter={(e) => {
        e.currentTarget.style.background = "var(--bg-hover)";
      }}
      onMouseLeave={(e) => {
        e.currentTarget.style.background = "transparent";
      }}
    >
      {entry.isDir ? (
        <Folder size={12} style={{ flexShrink: 0, color: "var(--fill-tertiary)" }} />
      ) : (
        <File size={12} style={{ flexShrink: 0, color: "var(--fill-tertiary)" }} />
      )}
      <span
        style={{
          flex: 1,
          overflow: "hidden",
          textOverflow: "ellipsis",
          whiteSpace: "nowrap",
          color: "var(--fill-primary)",
        }}
      >
        {entry.name}
      </span>
    </div>
  );
});

function FileListPanel({
  workDir,
  artifacts,
  activeFilePath,
  browseActive,
  onToggleCollapse,
  onOpenFile,
  onBrowseActivate,
}: Omit<FileListSidebarProps, "collapsed" | "overlayMode" | "overlayOpen" | "onOpenOverlay">) {
  const { t } = useTranslation("sidebar");
  const [browsePath, setBrowsePath] = useState<string | null>(null);
  const [entries, setEntries] = useState<DirEntry[]>([]);
  const [loading, setLoading] = useState(false);
  const [dirError, setDirError] = useState<string | null>(null);

  useEffect(() => {
    if (!browseActive || !workDir) {
      setBrowsePath(null);
      setEntries([]);
      return;
    }
    setBrowsePath(workDir);
  }, [browseActive, workDir]);

  useEffect(() => {
    if (!browsePath || !workDir) return;
    let cancelled = false;
    setLoading(true);
    setDirError(null);
    void listDirectory(browsePath, workDir)
      .then((list) => {
        if (!cancelled) setEntries(list.slice(0, MAX_DIR_ENTRIES));
      })
      .catch(() => {
        if (!cancelled) {
          setEntries([]);
          setDirError(t("dirLoadError", { defaultValue: "Unable to load directory" }));
        }
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [browsePath, workDir]);

  const handleOpenArtifact = useCallback(
    (path: string) => {
      if (!workDir) return;
      onOpenFile(resolveFilePath(path, workDir));
    },
    [onOpenFile, workDir],
  );

  const uniqueArtifacts = artifacts.filter(
    (a, i, arr) => arr.findIndex((b) => b.path === a.path) === i,
  );

  return (
    <div
      data-file-list-overlay=""
      style={{
        display: "flex",
        flexDirection: "column",
        height: "100%",
        minHeight: 0,
        background: "var(--bg-primary)",
      }}
    >
      <div
        style={{
          display: "flex",
          alignItems: "center",
          justifyContent: "space-between",
          padding: "6px 8px",
          borderBottom: "1px solid var(--border-primary)",
          flexShrink: 0,
        }}
      >
        <span style={{ fontSize: 11, fontWeight: 600, color: "var(--fill-secondary)" }}>
          {t("filesListTitle", { defaultValue: "Files" })}
        </span>
        <button
          type="button"
          style={{
            display: "flex",
            alignItems: "center",
            justifyContent: "center",
            width: 22,
            height: 22,
            borderRadius: 4,
            border: "none",
            background: "transparent",
            color: "var(--fill-tertiary)",
            cursor: "pointer",
          }}
          title={t("collapseList", { defaultValue: "Collapse" })}
          onClick={onToggleCollapse}
          onMouseEnter={(e) => {
            e.currentTarget.style.background = "var(--bg-hover)";
          }}
          onMouseLeave={(e) => {
            e.currentTarget.style.background = "transparent";
          }}
        >
          <CaretLeft size={12} weight="bold" />
        </button>
      </div>

      <div style={{ flex: 1, overflowY: "auto", minHeight: 0 }}>
        {uniqueArtifacts.length > 0 && (
          <>
            <div
              style={{
                padding: "6px 8px 4px",
                fontSize: 10,
                fontWeight: 600,
                color: "var(--fill-quaternary)",
                textTransform: "uppercase",
                letterSpacing: "0.04em",
              }}
            >
              {t("sessionArtifacts", { defaultValue: "Session" })}
            </div>
            {uniqueArtifacts.map((artifact) => (
              <ArtifactRow
                key={`${artifact.path}-${artifact.timestamp}`}
                artifact={artifact}
                active={activeFilePath === resolveFilePath(artifact.path, workDir ?? "")}
                onOpen={handleOpenArtifact}
              />
            ))}
          </>
        )}

        {workDir && (
          <>
            <div
              style={{
                padding: "8px 8px 4px",
                fontSize: 10,
                fontWeight: 600,
                color: "var(--fill-quaternary)",
                textTransform: "uppercase",
                letterSpacing: "0.04em",
                display: "flex",
                alignItems: "center",
                justifyContent: "space-between",
              }}
            >
              <span>{t("workDirBrowse", { defaultValue: "Browse" })}</span>
              {!browseActive && (
                <button
                  type="button"
                  style={{
                    fontSize: 10,
                    padding: "2px 6px",
                    borderRadius: 4,
                    border: "1px solid var(--border-primary)",
                    background: "var(--bg-secondary)",
                    color: "var(--fill-secondary)",
                    cursor: "pointer",
                  }}
                  onClick={onBrowseActivate}
                >
                  {t("browseWorkDir", { defaultValue: "Open" })}
                </button>
              )}
            </div>

            {browseActive && browsePath && (
              <>
                {browsePath !== workDir && (
                  <div
                    role="button"
                    tabIndex={0}
                    style={{
                      padding: "4px 8px",
                      fontSize: 10,
                      color: "var(--fill-tertiary)",
                      cursor: "pointer",
                    }}
                    onClick={() => {
                      const parent = browsePath.slice(0, browsePath.lastIndexOf("/"));
                      setBrowsePath(parent || workDir);
                    }}
                  >
                    ..
                  </div>
                )}
                {loading && (
                  <div style={{ padding: "8px", fontSize: 11, color: "var(--fill-quaternary)" }}>
                    {t("loading", { defaultValue: "Loading…" })}
                  </div>
                )}
                {dirError && !loading && (
                  <div style={{ padding: "8px", fontSize: 11, color: "var(--red-text)" }}>
                    {dirError}
                  </div>
                )}
                {!loading && !dirError &&
                  entries.map((entry) => (
                    <DirRow
                      key={entry.name}
                      entry={entry}
                      parentPath={browsePath}
                      onOpenFile={onOpenFile}
                      onNavigate={setBrowsePath}
                    />
                  ))}
              </>
            )}
          </>
        )}
      </div>
    </div>
  );
}

export const FileListSidebar = memo(function FileListSidebar(props: FileListSidebarProps) {
  const { collapsed, overlayMode, overlayOpen, onOpenOverlay, onToggleCollapse } = props;

  if (collapsed) {
    return (
      <>
        <div
          data-file-list-strip=""
          style={{
            width: FILE_LIST_COLLAPSED,
            minWidth: FILE_LIST_COLLAPSED,
            flexShrink: 0,
            display: "flex",
            flexDirection: "column",
            alignItems: "center",
            borderRight: "1px solid var(--border-primary)",
            background: "var(--bg-secondary)",
            paddingTop: 8,
          }}
        >
          <button
            type="button"
            style={{
              display: "flex",
              alignItems: "center",
              justifyContent: "center",
              width: 28,
              height: 28,
              borderRadius: 5,
              border: "none",
              background: overlayOpen ? "var(--bg-hover)" : "transparent",
              color: "var(--fill-secondary)",
              cursor: "pointer",
            }}
            title="Files"
            onClick={() => {
              if (overlayMode) {
                onOpenOverlay();
              } else {
                onToggleCollapse();
              }
            }}
            onMouseEnter={(e) => {
              e.currentTarget.style.background = "var(--bg-hover)";
            }}
            onMouseLeave={(e) => {
              if (!overlayOpen) e.currentTarget.style.background = "transparent";
            }}
          >
            <FolderOpen size={16} />
          </button>
          {!overlayMode && (
            <button
              type="button"
              style={{
                display: "flex",
                alignItems: "center",
                justifyContent: "center",
                width: 28,
                height: 28,
                borderRadius: 5,
                border: "none",
                background: "transparent",
                color: "var(--fill-quaternary)",
                cursor: "pointer",
                marginTop: 4,
              }}
              title="Expand"
              onClick={onToggleCollapse}
              onMouseEnter={(e) => {
                e.currentTarget.style.background = "var(--bg-hover)";
              }}
              onMouseLeave={(e) => {
                e.currentTarget.style.background = "transparent";
              }}
            >
              <CaretRight size={14} weight="bold" />
            </button>
          )}
        </div>

        {overlayMode && overlayOpen && (
          <div
            style={{
              position: "absolute",
              left: FILE_LIST_COLLAPSED,
              top: 0,
              bottom: 0,
              width: FILE_LIST_WIDTH,
              zIndex: 20,
              boxShadow: "2px 0 8px rgba(0,0,0,0.15)",
              borderRight: "1px solid var(--border-primary)",
            }}
          >
            <FileListPanel {...props} />
          </div>
        )}
      </>
    );
  }

  return (
    <div
      style={{
        width: FILE_LIST_WIDTH,
        minWidth: FILE_LIST_WIDTH,
        flexShrink: 0,
        display: "flex",
        flexDirection: "column",
        borderRight: "1px solid var(--border-primary)",
        minHeight: 0,
      }}
    >
      <FileListPanel {...props} />
    </div>
  );
});

