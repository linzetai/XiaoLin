import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Plus, Minus, ArrowCounterClockwise } from "@phosphor-icons/react";
import { useGitStore } from "../../lib/stores";
import type { FileChange, DiffHunk, DiffLine } from "../../../../xiaolin-protocol/generated/protocol";

export function ReviewTabContent() {
  const { t } = useTranslation("chat");
  const status = useGitStore((s) => s.status);
  const selectedFile = useGitStore((s) => s.selectedFile);
  const selectedDiff = useGitStore((s) => s.selectedDiff);
  const selectFile = useGitStore((s) => s.selectFile);
  const stageFiles = useGitStore((s) => s.stageFiles);
  const unstageFiles = useGitStore((s) => s.unstageFiles);

  if (!status?.isGitRepo) {
    return (
      <div className="flex h-full items-center justify-center" style={{ color: "var(--fill-quaternary)" }}>
        <p className="text-sm">{t("review_notGitRepo")}</p>
      </div>
    );
  }

  const hasChanges = status.staged.length > 0 || status.unstaged.length > 0 || status.untracked.length > 0;

  if (!hasChanges) {
    return (
      <div className="flex h-full items-center justify-center" style={{ color: "var(--fill-quaternary)" }}>
        <p className="text-sm">{t("review_noChanges")}</p>
      </div>
    );
  }

  return (
    <div className="flex h-full flex-col overflow-hidden">
      <div className="flex-1 overflow-y-auto px-3 py-2">
        {status.staged.length > 0 && (
          <FileGroup
            title={t("review_staged")}
            files={status.staged}
            selectedFile={selectedFile}
            onSelect={(path) => selectFile(path, true)}
            action="unstage"
            onAction={(path) => unstageFiles([path])}
          />
        )}
        {status.unstaged.length > 0 && (
          <FileGroup
            title={t("review_unstaged")}
            files={status.unstaged}
            selectedFile={selectedFile}
            onSelect={(path) => selectFile(path, false)}
            action="stage"
            onAction={(path) => stageFiles([path])}
          />
        )}
        {status.untracked.length > 0 && (
          <div className="mt-2">
            <div className="mb-1 text-[11px] font-medium uppercase tracking-wide" style={{ color: "var(--fill-tertiary)" }}>
              {t("review_untracked")}
            </div>
            {status.untracked.map((path) => (
              <div key={path} className="flex items-center gap-1 rounded px-2 py-0.5 text-[12px]" style={{ color: "var(--fill-secondary)" }}>
                <span className="flex-1 truncate">{path}</span>
                <button
                  className="shrink-0 rounded p-0.5 hover:bg-[var(--bg-hover)]"
                  onClick={() => stageFiles([path])}
                  title={t("review_stage")}
                >
                  <Plus size={12} />
                </button>
              </div>
            ))}
          </div>
        )}

        {selectedFile && selectedDiff.length > 0 && (
          <div className="mt-3 border-t pt-2" style={{ borderColor: "var(--separator)" }}>
            <div className="mb-1 text-[11px] font-medium" style={{ color: "var(--fill-tertiary)" }}>
              {selectedFile}
            </div>
            <DiffView hunks={selectedDiff} />
          </div>
        )}
      </div>
    </div>
  );
}

export function ReviewTabFooter() {
  const { t } = useTranslation("chat");
  const status = useGitStore((s) => s.status);
  const stageFiles = useGitStore((s) => s.stageFiles);
  const revertFiles = useGitStore((s) => s.revertFiles);
  const [showConfirm, setShowConfirm] = useState(false);

  if (!status?.isGitRepo) return null;

  const hasUnstaged = status.unstaged.length > 0 || status.untracked.length > 0;

  return (
    <div className="flex items-center gap-2 px-3 py-2">
      {hasUnstaged && (
        <button
          className="rounded px-2 py-1 text-[11px] font-medium transition-colors hover:bg-[var(--bg-hover)]"
          style={{ color: "var(--accent-primary)" }}
          onClick={() => stageFiles()}
        >
          <Plus size={10} className="mr-0.5 inline" /> {t("review_stageAll")}
        </button>
      )}
      {hasUnstaged && (
        <>
          {showConfirm ? (
            <div className="flex items-center gap-1">
              <span className="text-[11px]" style={{ color: "var(--fill-warning)" }}>{t("review_confirmRevert")}</span>
              <button
                className="rounded px-1.5 py-0.5 text-[10px] font-medium hover:bg-[var(--bg-hover)]"
                style={{ color: "var(--fill-danger)" }}
                onClick={() => {
                  const allFiles = [...status.unstaged.map((f) => f.path), ...status.untracked];
                  revertFiles(allFiles);
                  setShowConfirm(false);
                }}
              >
                {t("confirm", { ns: "common" })}
              </button>
              <button
                className="rounded px-1.5 py-0.5 text-[10px] hover:bg-[var(--bg-hover)]"
                style={{ color: "var(--fill-tertiary)" }}
                onClick={() => setShowConfirm(false)}
              >
                {t("cancel", { ns: "common" })}
              </button>
            </div>
          ) : (
            <button
              className="rounded px-2 py-1 text-[11px] font-medium transition-colors hover:bg-[var(--bg-hover)]"
              style={{ color: "var(--fill-danger)" }}
              onClick={() => setShowConfirm(true)}
            >
              <ArrowCounterClockwise size={10} className="mr-0.5 inline" /> {t("review_revertAll")}
            </button>
          )}
        </>
      )}
    </div>
  );
}

function FileGroup({
  title,
  files,
  selectedFile,
  onSelect,
  action,
  onAction,
}: {
  title: string;
  files: FileChange[];
  selectedFile: string | null;
  onSelect: (path: string) => void;
  action: "stage" | "unstage";
  onAction: (path: string) => void;
}) {
  const { t } = useTranslation("chat");
  const ActionIcon = action === "stage" ? Plus : Minus;

  return (
    <div className="mt-1">
      <div className="mb-1 text-[11px] font-medium uppercase tracking-wide" style={{ color: "var(--fill-tertiary)" }}>
        {title} ({files.length})
      </div>
      {files.map((file) => (
        <div
          key={file.path}
          className="group flex cursor-pointer items-center gap-1 rounded px-2 py-0.5 text-[12px] transition-colors hover:bg-[var(--bg-hover)]"
          style={{
            color: "var(--fill-secondary)",
            background: selectedFile === file.path ? "var(--bg-hover)" : undefined,
          }}
          onClick={() => onSelect(file.path)}
        >
          <StatusBadge status={file.status} />
          <span className="flex-1 truncate">{file.path}</span>
          <button
            className="shrink-0 rounded p-0.5 opacity-0 transition-opacity group-hover:opacity-100 hover:bg-[var(--bg-tertiary)]"
            onClick={(e) => { e.stopPropagation(); onAction(file.path); }}
            title={action === "stage" ? t("review_stage") : t("review_unstage")}
          >
            <ActionIcon size={12} />
          </button>
        </div>
      ))}
    </div>
  );
}

function StatusBadge({ status }: { status: string }) {
  const colors: Record<string, string> = {
    added: "var(--fill-success)",
    modified: "var(--accent-primary)",
    deleted: "var(--fill-danger)",
    renamed: "var(--fill-warning)",
  };
  const labels: Record<string, string> = {
    added: "A",
    modified: "M",
    deleted: "D",
    renamed: "R",
    copied: "C",
    typeChanged: "T",
    unmerged: "U",
  };
  return (
    <span
      className="inline-flex h-4 w-4 shrink-0 items-center justify-center rounded text-[9px] font-bold"
      style={{ color: colors[status] ?? "var(--fill-tertiary)" }}
    >
      {labels[status] ?? "?"}
    </span>
  );
}

function DiffView({ hunks }: { hunks: DiffHunk[] }) {
  return (
    <div className="overflow-x-auto rounded text-[11px] font-mono" style={{ background: "var(--bg-tertiary)" }}>
      {hunks.map((hunk, i) => (
        <div key={i} className="border-b last:border-b-0" style={{ borderColor: "var(--separator)" }}>
          <div className="px-2 py-0.5 text-[10px]" style={{ color: "var(--fill-quaternary)", background: "var(--bg-secondary)" }}>
            {hunk.header}
          </div>
          {hunk.lines.map((line, j) => (
            <DiffLineRow key={j} line={line} />
          ))}
        </div>
      ))}
    </div>
  );
}

function DiffLineRow({ line }: { line: DiffLine }) {
  const bg =
    line.kind === "add"
      ? "rgba(46, 160, 67, 0.08)"
      : line.kind === "delete"
        ? "rgba(248, 81, 73, 0.08)"
        : "transparent";
  const color =
    line.kind === "add"
      ? "var(--fill-success)"
      : line.kind === "delete"
        ? "var(--fill-danger)"
        : "var(--fill-secondary)";
  const prefix = line.kind === "add" ? "+" : line.kind === "delete" ? "-" : " ";

  return (
    <div className="flex" style={{ background: bg }}>
      <span className="inline-block w-4 shrink-0 select-none text-right" style={{ color: "var(--fill-quaternary)" }}>
        {prefix}
      </span>
      <span className="flex-1 whitespace-pre-wrap break-all px-1" style={{ color }}>
        {line.content}
      </span>
    </div>
  );
}
