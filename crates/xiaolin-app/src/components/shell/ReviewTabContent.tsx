import { useState, useCallback } from "react";
import {
  CaretDown, CaretRight, ArrowCounterClockwise,
  Plus as PlusIcon, Minus, GitBranch as GitBranchIcon,
  Check, ArrowUp, ArrowDown,
} from "@phosphor-icons/react";
import { useGitStore } from "../../lib/stores";
import type { FileChange, DiffHunk, DiffLine, FileStatus } from "../../../../xiaolin-protocol/generated/protocol";
import * as transport from "../../lib/transport";
import { useProjectStore } from "../../lib/stores/project-store";

const statusColorMap: Record<FileStatus, string> = {
  added: "var(--green-text)",
  modified: "var(--blue-text, #60a5fa)",
  deleted: "var(--red-text)",
  renamed: "var(--purple-text, #a78bfa)",
  copied: "var(--fill-secondary)",
  typeChanged: "var(--fill-secondary)",
  unmerged: "var(--orange-text, #fb923c)",
};

const statusLabelMap: Record<FileStatus, string> = {
  added: "A",
  modified: "M",
  deleted: "D",
  renamed: "R",
  copied: "C",
  typeChanged: "T",
  unmerged: "U",
};

function FileRow({
  file,
  active,
  onClick,
  actionIcon,
  onAction,
}: {
  file: FileChange;
  active: boolean;
  onClick: () => void;
  actionIcon?: "stage" | "unstage";
  onAction?: () => void;
}) {
  const fileName = file.path.split("/").pop() ?? file.path;
  const dirPath = file.path.includes("/") ? file.path.substring(0, file.path.lastIndexOf("/")) : "";

  return (
    <div
      style={{
        display: "flex",
        alignItems: "center",
        padding: "5px 10px 5px 24px",
        fontSize: 12,
        fontFamily: "var(--font-mono)",
        cursor: "pointer",
        background: active ? "var(--bg-hover)" : "transparent",
        transition: "background 0.1s",
      }}
      onClick={onClick}
      onMouseEnter={(e) => { if (!active) e.currentTarget.style.background = "var(--bg-hover)"; }}
      onMouseLeave={(e) => { e.currentTarget.style.background = active ? "var(--bg-hover)" : "transparent"; }}
    >
      <span style={{
        fontSize: 10, fontWeight: 700, width: 14, textAlign: "center", marginRight: 6, flexShrink: 0,
        color: statusColorMap[file.status] ?? "var(--fill-tertiary)",
      }}>
        {statusLabelMap[file.status] ?? "?"}
      </span>
      <span style={{ flex: 1, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap", color: "var(--fill-primary)" }}>
        {fileName}
      </span>
      {dirPath && (
        <span style={{ fontSize: 10, color: "var(--fill-quaternary)", marginLeft: 6, flexShrink: 0 }}>
          {dirPath}
        </span>
      )}
      {actionIcon && onAction && (
        <button
          type="button"
          onClick={(e) => { e.stopPropagation(); onAction(); }}
          style={{
            display: "flex", alignItems: "center", justifyContent: "center",
            width: 18, height: 18, borderRadius: 3, border: "none",
            background: "transparent", color: "var(--fill-tertiary)",
            cursor: "pointer", marginLeft: 4, flexShrink: 0,
          }}
          title={actionIcon === "stage" ? "Stage" : "Unstage"}
        >
          {actionIcon === "stage" ? <PlusIcon size={11} /> : <Minus size={11} />}
        </button>
      )}
    </div>
  );
}

function SectionHeader({
  title, count, expanded, onToggle, actions,
}: {
  title: string;
  count: number;
  expanded: boolean;
  onToggle: () => void;
  actions?: React.ReactNode;
}) {
  return (
    <div
      style={{
        display: "flex", alignItems: "center", gap: 5, padding: "6px 10px",
        fontSize: 11, fontWeight: 600, color: "var(--fill-secondary)",
        cursor: "pointer", userSelect: "none",
      }}
      onClick={onToggle}
    >
      {expanded
        ? <CaretDown size={12} weight="bold" />
        : <CaretRight size={12} weight="bold" />
      }
      <span>{title}</span>
      <span style={{
        fontSize: 10, fontFamily: "var(--font-mono)",
        background: "var(--bg-hover)", padding: "1px 5px", borderRadius: 4,
        color: "var(--fill-tertiary)",
      }}>
        {count}
      </span>
      {actions && (
        <div style={{ marginLeft: "auto", display: "flex", gap: 4 }} onClick={(e) => e.stopPropagation()}>
          {actions}
        </div>
      )}
    </div>
  );
}

function SmallBtn({ onClick, title, children, variant }: {
  onClick: () => void; title: string; children: React.ReactNode;
  variant?: "danger" | "default";
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      title={title}
      style={{
        display: "flex", alignItems: "center", gap: 3,
        padding: "2px 6px", borderRadius: 4,
        border: "none", background: "transparent",
        color: variant === "danger" ? "var(--red-text)" : "var(--fill-tertiary)",
        cursor: "pointer", fontSize: 10, fontWeight: 500,
      }}
    >
      {children}
    </button>
  );
}

function DiffLineRow({ line }: { line: DiffLine }) {
  const bgMap: Record<string, string> = { add: "rgba(46,160,67,0.08)", delete: "rgba(248,81,73,0.08)", context: "transparent" };
  const colorMap: Record<string, string> = { add: "var(--green-text)", delete: "var(--red-text)", context: "var(--fill-secondary)" };
  return (
    <div style={{
      fontFamily: "var(--font-mono)", fontSize: 11, padding: "1px 8px",
      background: bgMap[line.kind] ?? "transparent",
      color: colorMap[line.kind] ?? "var(--fill-primary)",
      whiteSpace: "pre-wrap", wordBreak: "break-all", lineHeight: 1.5,
    }}>
      {line.kind === "add" ? "+" : line.kind === "delete" ? "-" : " "}{line.content}
    </div>
  );
}

function DiffView({ hunks, file }: { hunks: DiffHunk[]; file: string }) {
  if (hunks.length === 0) {
    return (
      <div style={{ padding: 16, fontSize: 12, color: "var(--fill-tertiary)", textAlign: "center" }}>
        No diff available for {file.split("/").pop()}
      </div>
    );
  }
  return (
    <div style={{ overflow: "auto" }}>
      <div style={{ padding: "6px 10px", fontSize: 11, color: "var(--fill-tertiary)", fontFamily: "var(--font-mono)", borderBottom: "1px solid var(--border-shell-subtle)" }}>
        {file}
      </div>
      {hunks.map((hunk, i) => (
        <div key={i}>
          <div style={{ fontSize: 10, color: "var(--fill-tertiary)", padding: "3px 8px", background: "var(--bg-hover)", fontFamily: "var(--font-mono)" }}>
            @@ -{hunk.oldStart},{hunk.oldLines} +{hunk.newStart},{hunk.newLines} @@
          </div>
          {hunk.lines.map((line, j) => <DiffLineRow key={j} line={line} />)}
        </div>
      ))}
    </div>
  );
}

function CommitBox() {
  const stagedCount = useGitStore((s) => s.status?.staged?.length ?? 0);
  const commitChanges = useGitStore((s) => s.commitChanges);
  const clearSelection = useGitStore((s) => s.clearSelection);
  const refresh = useGitStore((s) => s.refresh);
  const [msg, setMsg] = useState("");
  const [committing, setCommitting] = useState(false);
  const [justCommitted, setJustCommitted] = useState(false);

  const handleCommit = useCallback(async () => {
    if (!msg.trim() || stagedCount === 0) return;
    setCommitting(true);
    try {
      await commitChanges(msg.trim());
      setMsg("");
      clearSelection();
      refresh();
      setJustCommitted(true);
      setTimeout(() => setJustCommitted(false), 2500);
    } finally {
      setCommitting(false);
    }
  }, [msg, stagedCount, commitChanges, clearSelection, refresh]);

  if (stagedCount === 0 && !justCommitted) return null;

  if (justCommitted) {
    return (
      <div style={{ padding: "10px", borderTop: "1px solid var(--border-shell-subtle)", textAlign: "center" }}>
        <span style={{ fontSize: 11, color: "var(--green-text)", fontWeight: 500 }}>
          ✓ Committed successfully
        </span>
      </div>
    );
  }

  return (
    <div style={{ padding: "8px 10px", borderTop: "1px solid var(--border-shell-subtle)" }}>
      <div style={{ display: "flex", gap: 6 }}>
        <textarea
          autoFocus
          value={msg}
          onChange={(e) => setMsg(e.target.value)}
          onKeyDown={(e) => { if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) handleCommit(); }}
          placeholder="Commit message… (⌘↩ to commit)"
          rows={2}
          style={{
            flex: 1, resize: "none", borderRadius: 6,
            border: "1px solid var(--border-shell)", background: "var(--bg-primary)",
            color: "var(--fill-primary)", padding: "6px 8px",
            fontSize: 11, fontFamily: "var(--font-mono)",
            outline: "none", lineHeight: 1.4,
          }}
        />
      </div>
      <div style={{ display: "flex", justifyContent: "flex-end", marginTop: 6 }}>
        <button
          type="button"
          onClick={handleCommit}
          disabled={!msg.trim() || committing}
          style={{
            display: "flex", alignItems: "center", gap: 4,
            padding: "5px 12px", borderRadius: 6,
            border: "none",
            background: msg.trim() ? "var(--green-text)" : "var(--bg-hover)",
            color: msg.trim() ? "#fff" : "var(--fill-tertiary)",
            fontSize: 11, fontWeight: 600, cursor: msg.trim() ? "pointer" : "not-allowed",
            transition: "background 0.12s",
          }}
        >
          <Check size={12} weight="bold" />
          {committing ? "Committing..." : `Commit (${stagedCount})`}
        </button>
      </div>
    </div>
  );
}

function BranchBar() {
  const status = useGitStore((s) => s.status);
  const branch = useGitStore((s) => s.currentBranch);

  if (!status?.isGitRepo) return null;

  const staged = status.staged?.length ?? 0;
  const unstaged = (status.unstaged?.length ?? 0) + (status.untracked?.length ?? 0);

  return (
    <div style={{
      display: "flex", alignItems: "center", gap: 8, padding: "8px 10px",
      borderBottom: "1px solid var(--border-shell-subtle)",
      fontSize: 12,
    }}>
      <span style={{ display: "flex", alignItems: "center", gap: 4, color: "var(--fill-secondary)" }}>
        <GitBranchIcon size={13} />
        <span style={{ fontWeight: 500 }}>{branch || "HEAD"}</span>
      </span>
      {status.ahead > 0 && (
        <span style={{ display: "flex", alignItems: "center", gap: 2, fontSize: 10, color: "var(--green-text)" }}>
          <ArrowUp size={10} />{status.ahead}
        </span>
      )}
      {status.behind > 0 && (
        <span style={{ display: "flex", alignItems: "center", gap: 2, fontSize: 10, color: "var(--red-text)" }}>
          <ArrowDown size={10} />{status.behind}
        </span>
      )}
      <div style={{ flex: 1 }} />
      {(staged + unstaged) > 0 && (
        <span style={{ fontSize: 10, fontFamily: "var(--font-mono)", color: "var(--fill-tertiary)" }}>
          {staged + unstaged} file{(staged + unstaged) > 1 ? "s" : ""} changed
        </span>
      )}
    </div>
  );
}

function GitInitView() {
  const activeProjectId = useProjectStore((s) => s.activeProjectId);
  const [initing, setIniting] = useState(false);

  const handleInit = async () => {
    if (!activeProjectId) return;
    setIniting(true);
    try {
      await transport.gitInit(activeProjectId);
      useGitStore.getState().refresh();
    } catch {
      /* ignore */
    } finally {
      setIniting(false);
    }
  };

  return (
    <div style={{ display: "flex", flexDirection: "column", alignItems: "center", justifyContent: "center", height: "100%", gap: 12, padding: 24 }}>
      <GitBranchIcon size={28} weight="light" style={{ color: "var(--fill-tertiary)" }} />
      <span style={{ fontSize: 12, color: "var(--fill-secondary)", textAlign: "center" }}>
        Not a Git repository
      </span>
      <button
        type="button"
        onClick={handleInit}
        disabled={initing}
        style={{
          display: "flex", alignItems: "center", gap: 6,
          padding: "6px 12px", borderRadius: 6,
          border: "1px solid var(--border-shell)",
          background: "var(--bg-hover)", color: "var(--fill-primary)",
          fontSize: 11, fontWeight: 500, cursor: "pointer",
        }}
      >
        <GitBranchIcon size={12} />
        {initing ? "Initializing..." : "Initialize Git"}
      </button>
    </div>
  );
}

function NoProjectView() {
  return (
    <div style={{ display: "flex", flexDirection: "column", alignItems: "center", justifyContent: "center", height: "100%", gap: 8, padding: 24 }}>
      <span style={{ fontSize: 12, color: "var(--fill-tertiary)", textAlign: "center" }}>
        Set a working directory to view changes
      </span>
    </div>
  );
}

export function ReviewTabContent() {
  const status = useGitStore((s) => s.status);
  const selectedDiff = useGitStore((s) => s.selectedDiff);
  const selectedFile = useGitStore((s) => s.selectedFile);
  const selectFile = useGitStore((s) => s.selectFile);
  const clearSelection = useGitStore((s) => s.clearSelection);
  const stageFiles = useGitStore((s) => s.stageFiles);
  const unstageFiles = useGitStore((s) => s.unstageFiles);
  const revertFiles = useGitStore((s) => s.revertFiles);
  const activeProjectId = useProjectStore((s) => s.activeProjectId);

  const [stagedExpanded, setStagedExpanded] = useState(true);
  const [unstagedExpanded, setUnstagedExpanded] = useState(true);
  const [confirmRevert, setConfirmRevert] = useState(false);

  if (!activeProjectId) return <NoProjectView />;
  if (!status) return <NoProjectView />;
  if (!status.isGitRepo) return <GitInitView />;

  const staged = status.staged ?? [];
  const unstaged = status.unstaged ?? [];
  const untracked = (status.untracked ?? []).map((p: string): FileChange => ({
    path: p, status: "added",
  }));
  const allUnstaged = [...unstaged, ...untracked];
  const allPaths = [...allUnstaged.map((f) => f.path)];

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%" }}>
      <BranchBar />

      {/* File list area */}
      <div style={{
        flex: selectedFile ? "0 0 auto" : 1,
        overflow: "auto",
        maxHeight: selectedFile ? "45%" : undefined,
      }}>
        {staged.length > 0 && (
          <>
            <SectionHeader
              title="Staged"
              count={staged.length}
              expanded={stagedExpanded}
              onToggle={() => setStagedExpanded(!stagedExpanded)}
              actions={
                <SmallBtn onClick={() => unstageFiles()} title="Unstage all">
                  <Minus size={10} /> All
                </SmallBtn>
              }
            />
            {stagedExpanded && staged.map((f) => (
              <FileRow
                key={`s-${f.path}`}
                file={f}
                active={selectedFile === f.path}
                onClick={() => selectFile(f.path, true)}
                actionIcon="unstage"
                onAction={() => unstageFiles([f.path])}
              />
            ))}
          </>
        )}

        {allUnstaged.length > 0 && (
          <>
            <SectionHeader
              title="Changes"
              count={allUnstaged.length}
              expanded={unstagedExpanded}
              onToggle={() => setUnstagedExpanded(!unstagedExpanded)}
              actions={
                <>
                  <SmallBtn onClick={() => stageFiles()} title="Stage all">
                    <PlusIcon size={10} /> All
                  </SmallBtn>
                  <SmallBtn
                    onClick={() => {
                      if (!confirmRevert) { setConfirmRevert(true); setTimeout(() => setConfirmRevert(false), 3000); return; }
                      revertFiles(allPaths);
                      setConfirmRevert(false);
                    }}
                    title="Revert all changes"
                    variant="danger"
                  >
                    <ArrowCounterClockwise size={10} /> {confirmRevert ? "Confirm?" : "Revert"}
                  </SmallBtn>
                </>
              }
            />
            {unstagedExpanded && allUnstaged.map((f) => (
              <FileRow
                key={`u-${f.path}`}
                file={f}
                active={selectedFile === f.path}
                onClick={() => selectFile(f.path, false)}
                actionIcon="stage"
                onAction={() => stageFiles([f.path])}
              />
            ))}
          </>
        )}

        {staged.length === 0 && allUnstaged.length === 0 && (
          <div style={{ padding: 24, textAlign: "center", fontSize: 12, color: "var(--fill-tertiary)" }}>
            Working tree clean
          </div>
        )}
      </div>

      {/* Diff detail view */}
      {selectedFile && (
        <div style={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "column", borderTop: "1px solid var(--border-shell-subtle)" }}>
          <div style={{
            display: "flex", alignItems: "center", padding: "4px 10px",
            fontSize: 11, color: "var(--fill-tertiary)",
            borderBottom: "1px solid var(--border-shell-subtle)",
          }}>
            <span style={{ flex: 1, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
              Diff: {selectedFile.split("/").pop()}
            </span>
            <button
              type="button"
              onClick={clearSelection}
              style={{ border: "none", background: "transparent", color: "var(--fill-tertiary)", cursor: "pointer", fontSize: 10, padding: "2px 4px" }}
            >
              ✕
            </button>
          </div>
          <div style={{ flex: 1, overflow: "auto" }}>
            <DiffView hunks={selectedDiff} file={selectedFile} />
          </div>
        </div>
      )}

      {/* Commit box */}
      <CommitBox />
    </div>
  );
}

export function ReviewTabFooter() {
  return null;
}
