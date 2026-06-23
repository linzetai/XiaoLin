import { memo, useCallback, useEffect, useState } from "react";
import {
  CaretRight,
  CaretDown,
  Folder,
  FolderOpen,
  File,
  FileTs,
  FileJs,
  FileJsx,
  FilePy,
  FileRs,
  FileSql,
  FileCss,
  FileHtml,
  FileVue,
  FileMd,
  FileImage,
} from "@phosphor-icons/react";
import { useTranslation } from "react-i18next";
import { listDirectory, type DirEntry } from "../../lib/transport";

const MAX_DIR_ENTRIES = 500;

type FileIcon = {
  icon: typeof File;
  color: string;
};

const EXT_ICON_MAP: Record<string, FileIcon> = {
  ts: { icon: FileTs, color: "#3178c6" },
  tsx: { icon: FileJsx, color: "#3178c6" },
  js: { icon: FileJs, color: "#f7df1e" },
  jsx: { icon: FileJsx, color: "#61dafb" },
  mjs: { icon: FileJs, color: "#f7df1e" },
  py: { icon: FilePy, color: "#3776ab" },
  rs: { icon: FileRs, color: "#dea584" },
  sql: { icon: FileSql, color: "#e38c00" },
  css: { icon: FileCss, color: "#264de4" },
  scss: { icon: FileCss, color: "#cd6799" },
  less: { icon: FileCss, color: "#1d365d" },
  html: { icon: FileHtml, color: "#e34c26" },
  htm: { icon: FileHtml, color: "#e34c26" },
  vue: { icon: FileVue, color: "#42b883" },
  md: { icon: FileMd, color: "#755838" },
  mdx: { icon: FileMd, color: "#755838" },
  png: { icon: FileImage, color: "#a855f7" },
  jpg: { icon: FileImage, color: "#a855f7" },
  jpeg: { icon: FileImage, color: "#a855f7" },
  gif: { icon: FileImage, color: "#a855f7" },
  webp: { icon: FileImage, color: "#a855f7" },
  svg: { icon: FileImage, color: "#ffb13b" },
};

export function getFileIcon(name: string): FileIcon {
  const ext = name.includes(".") ? (name.split(".").pop()?.toLowerCase() ?? "") : "";
  return EXT_ICON_MAP[ext] ?? { icon: File, color: "var(--fill-tertiary)" };
}

interface TreeNodeProps {
  entry: DirEntry;
  parentPath: string;
  workDir: string;
  depth: number;
  onOpenFile: (path: string) => void;
}

const TreeNode = memo(function TreeNode({
  entry,
  parentPath,
  workDir,
  depth,
  onOpenFile,
}: TreeNodeProps) {
  const { t } = useTranslation("fileViewer");
  const [expanded, setExpanded] = useState(false);
  const [children, setChildren] = useState<DirEntry[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState(false);
  const [loaded, setLoaded] = useState(false);
  const [truncated, setTruncated] = useState(false);

  const fullPath = `${parentPath.replace(/\/+$/, "")}/${entry.name}`;
  const paddingLeft = 8 + depth * 12;

  const toggleExpand = useCallback(() => {
    if (!entry.isDir) return;
    setExpanded((prev) => !prev);
  }, [entry.isDir]);

  useEffect(() => {
    if (!expanded || !entry.isDir || loaded) return;
    let cancelled = false;
    setLoading(true);
    setError(false);
    void listDirectory(fullPath, workDir)
      .then((list) => {
        if (!cancelled) {
          setTruncated(list.length > MAX_DIR_ENTRIES);
          setChildren(list.slice(0, MAX_DIR_ENTRIES));
          setLoaded(true);
        }
      })
      .catch(() => {
        if (!cancelled) setError(true);
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [expanded, entry.isDir, loaded, fullPath, workDir]);

  const handleClick = useCallback(() => {
    if (entry.isDir) {
      toggleExpand();
    } else {
      onOpenFile(fullPath);
    }
  }, [entry.isDir, fullPath, onOpenFile, toggleExpand]);

  const fileIconInfo = entry.isDir ? null : getFileIcon(entry.name);

  return (
    <>
      <div
        role="treeitem"
        tabIndex={0}
        aria-expanded={entry.isDir ? expanded : undefined}
        style={{
          display: "flex",
          alignItems: "center",
          gap: 3,
          padding: `3px 6px 3px ${paddingLeft}px`,
          fontSize: 11,
          fontFamily: "var(--font-mono)",
          cursor: "pointer",
          transition: "background 0.08s",
          whiteSpace: "nowrap",
        }}
        onClick={handleClick}
        onKeyDown={(e) => {
          if (e.key === "Enter" || e.key === " ") {
            e.preventDefault();
            handleClick();
          }
          if (entry.isDir) {
            if (e.key === "ArrowRight" && !expanded) {
              e.preventDefault();
              setExpanded(true);
            }
            if (e.key === "ArrowLeft" && expanded) {
              e.preventDefault();
              setExpanded(false);
            }
          }
        }}
        onMouseEnter={(e) => {
          e.currentTarget.style.background = "var(--bg-hover)";
        }}
        onMouseLeave={(e) => {
          e.currentTarget.style.background = "transparent";
        }}
      >
        {entry.isDir ? (
          <>
            {expanded ? (
              <CaretDown size={10} weight="fill" style={{ flexShrink: 0, color: "var(--fill-quaternary)" }} />
            ) : (
              <CaretRight size={10} weight="fill" style={{ flexShrink: 0, color: "var(--fill-quaternary)" }} />
            )}
            {expanded ? (
              <FolderOpen size={13} style={{ flexShrink: 0, color: "var(--fill-tertiary)" }} />
            ) : (
              <Folder size={13} style={{ flexShrink: 0, color: "var(--fill-tertiary)" }} />
            )}
          </>
        ) : (
          <>
            <span style={{ width: 10, flexShrink: 0 }} />
            {fileIconInfo && (
              <fileIconInfo.icon size={13} style={{ flexShrink: 0, color: fileIconInfo.color }} />
            )}
          </>
        )}
        <span
          style={{
            flex: 1,
            overflow: "hidden",
            textOverflow: "ellipsis",
            color: "var(--fill-primary)",
          }}
        >
          {entry.name}
        </span>
      </div>

      {entry.isDir && expanded && (
        <div role="group">
          {loading && (
            <div style={{ padding: `3px 6px 3px ${paddingLeft + 22}px`, fontSize: 10, color: "var(--fill-quaternary)" }}>
              {t("loading")}
            </div>
          )}
          {error && !loading && (
            <div style={{ padding: `3px 6px 3px ${paddingLeft + 22}px`, fontSize: 10, color: "var(--red-text)" }}>
              {t("loadNodeFailed")}
            </div>
          )}
          {!loading && !error && children.map((child) => (
            <TreeNode
              key={`${fullPath}/${child.name}`}
              entry={child}
              parentPath={fullPath}
              workDir={workDir}
              depth={depth + 1}
              onOpenFile={onOpenFile}
            />
          ))}
          {!loading && !error && truncated && (
            <div style={{ padding: `3px 6px 3px ${paddingLeft + 22}px`, fontSize: 10, color: "var(--fill-quaternary)", fontStyle: "italic" }}>
              {t("itemsTruncated", { count: MAX_DIR_ENTRIES })}
            </div>
          )}
        </div>
      )}
    </>
  );
});

export interface FileTreeProps {
  workDir: string;
  onOpenFile: (path: string) => void;
}

export const FileTree = memo(function FileTree({ workDir, onOpenFile }: FileTreeProps) {
  const { t } = useTranslation("fileViewer");
  const [entries, setEntries] = useState<DirEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState(false);
  const [truncated, setTruncated] = useState(false);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setError(false);
    void listDirectory(workDir, workDir)
      .then((list) => {
        if (!cancelled) {
          setTruncated(list.length > MAX_DIR_ENTRIES);
          setEntries(list.slice(0, MAX_DIR_ENTRIES));
        }
      })
      .catch(() => {
        if (!cancelled) setError(true);
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [workDir]);

  if (loading) {
    return (
      <div style={{ padding: "8px 12px", fontSize: 11, color: "var(--fill-quaternary)" }}>
        {t("loading")}
      </div>
    );
  }
  if (error) {
    return (
      <div style={{ padding: "8px 12px", fontSize: 11, color: "var(--red-text)" }}>
        {t("loadDirectoryFailed")}
      </div>
    );
  }
  if (entries.length === 0) {
    return (
      <div style={{ padding: "8px 12px", fontSize: 11, color: "var(--fill-quaternary)" }}>
        {t("emptyDirectory")}
      </div>
    );
  }

  return (
    <div role="tree" aria-label={t("fileTreeAriaLabel")}>
      {entries.map((entry) => (
        <TreeNode
          key={`${workDir}/${entry.name}`}
          entry={entry}
          parentPath={workDir}
          workDir={workDir}
          depth={0}
          onOpenFile={onOpenFile}
        />
      ))}
      {truncated && (
        <div style={{ padding: "3px 6px 3px 20px", fontSize: 10, color: "var(--fill-quaternary)", fontStyle: "italic" }}>
          {t("itemsTruncated", { count: MAX_DIR_ENTRIES })}
        </div>
      )}
    </div>
  );
});
