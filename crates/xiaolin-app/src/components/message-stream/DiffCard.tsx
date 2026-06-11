/**
 * Specialized renderer for edit_file tool results.
 * Compact file row with expandable inline diff.
 */

import { useState, useMemo } from "react";
import { useTranslation } from "react-i18next";
import { Plus, Minus, CaretRight } from "@phosphor-icons/react";
import { parseEditResult } from "./edit-result-utils";

function DiffStatBadge({ added, removed }: { added: number; removed: number }) {
  const { t } = useTranslation("chat");
  const total = added + removed;
  if (total === 0) return <span className="text-[10px]" style={{ color: "var(--fill-quaternary)" }}>{t("diff_noChange")}</span>;

  const maxDots = 5;
  const addDots = total > 0 ? Math.max(1, Math.round((added / total) * maxDots)) : 0;
  const remDots = total > 0 ? Math.max(total > 0 && removed > 0 ? 1 : 0, maxDots - addDots) : 0;

  return (
    <div className="flex items-center gap-1.5">
      {added > 0 && (
        <span className="flex items-center gap-0.5 text-[10px] font-medium tabular-nums" style={{ color: "var(--green, #48BB78)" }}>
          <Plus size={14} weight="bold" />
          {added}
        </span>
      )}
      {removed > 0 && (
        <span className="flex items-center gap-0.5 text-[10px] font-medium tabular-nums" style={{ color: "var(--red, #FC8181)" }}>
          <Minus size={14} weight="bold" />
          {removed}
        </span>
      )}
      <div className="flex gap-[2px]">
        {Array.from({ length: addDots }).map((_, i) => (
          <div key={`a${i}`} className="h-[6px] w-[6px] rounded-[1px]" style={{ background: "var(--green, #48BB78)" }} />
        ))}
        {Array.from({ length: remDots }).map((_, i) => (
          <div key={`r${i}`} className="h-[6px] w-[6px] rounded-[1px]" style={{ background: "var(--red, #FC8181)" }} />
        ))}
        {Array.from({ length: Math.max(0, maxDots - addDots - remDots) }).map((_, i) => (
          <div key={`n${i}`} className="h-[6px] w-[6px] rounded-[1px]" style={{ background: "var(--fill-quaternary)" }} />
        ))}
      </div>
    </div>
  );
}

function parseEditArgs(argsStr: string): { oldString?: string; newString?: string } | null {
  try {
    const args = JSON.parse(argsStr);
    return {
      oldString: args.old_string || args.oldString,
      newString: args.new_string || args.newString,
    };
  } catch { return null; }
}

const INITIAL_MAX = 12;

function InlineDiff({ oldStr, newStr }: { oldStr: string; newStr: string }) {
  const { t } = useTranslation("chat");
  const [showAll, setShowAll] = useState(false);

  const allLines = useMemo(() => {
    const oldLines = oldStr.split("\n");
    const newLines = newStr.split("\n");
    const lines: Array<{ type: "remove" | "add" | "context"; text: string }> = [];

    let oi = 0;
    let ni = 0;
    while (oi < oldLines.length || ni < newLines.length) {
      if (oi < oldLines.length && ni < newLines.length && oldLines[oi] === newLines[ni]) {
        lines.push({ type: "context", text: oldLines[oi] });
        oi++;
        ni++;
      } else if (oi < oldLines.length) {
        lines.push({ type: "remove", text: oldLines[oi] });
        oi++;
      } else if (ni < newLines.length) {
        lines.push({ type: "add", text: newLines[ni] });
        ni++;
      }
    }
    return lines;
  }, [oldStr, newStr]);

  const truncated = !showAll && allLines.length > INITIAL_MAX;
  const display = truncated ? allLines.slice(0, INITIAL_MAX) : allLines;
  const remaining = allLines.length - INITIAL_MAX;

  return (
    <pre
      className="overflow-x-auto rounded-md text-[11px] leading-[1.6]"
      style={{
        background: "var(--bg-primary)",
        border: "0.5px solid var(--separator)",
        fontFamily: '"SF Mono","Fira Code",Menlo,Monaco,monospace',
      }}
    >
      {display.map((line, i) => (
        <div
          key={i}
          className="px-2"
          style={{
            background:
              line.type === "add"
                ? "color-mix(in srgb, var(--green, #48BB78) 10%, transparent)"
                : line.type === "remove"
                  ? "color-mix(in srgb, var(--red, #FC8181) 10%, transparent)"
                  : "transparent",
            color:
              line.type === "add"
                ? "var(--green, #48BB78)"
                : line.type === "remove"
                  ? "var(--red, #FC8181)"
                  : "var(--fill-tertiary)",
          }}
        >
          <span className="mr-2 inline-block w-3 select-none text-right opacity-60">
            {line.type === "add" ? "+" : line.type === "remove" ? "-" : " "}
          </span>
          {line.text || " "}
        </div>
      ))}
      {truncated && (
        <button
          type="button"
          onClick={() => setShowAll(true)}
          className="w-full cursor-pointer px-2 py-1 text-left text-[11px] transition-colors hover:underline"
          style={{ color: "var(--tint)" }}
        >
          {t("diff_expandRemaining", { count: remaining })}
        </button>
      )}
      {showAll && allLines.length > INITIAL_MAX && (
        <button
          type="button"
          onClick={() => setShowAll(false)}
          className="w-full cursor-pointer px-2 py-1 text-left text-[11px] transition-colors hover:underline"
          style={{ color: "var(--tint)" }}
        >
          {t("collapse", { ns: "common" })}
        </button>
      )}
    </pre>
  );
}

export function DiffCard({ result, args }: { result: string; args?: string }) {
  const { t } = useTranslation("chat");
  const [diffOpen, setDiffOpen] = useState(false);
  const editResult = parseEditResult(result);
  if (!editResult) return null;

  const fileName = editResult.path.split("/").pop() || editResult.path;
  const dir = editResult.path.includes("/")
    ? editResult.path.slice(0, editResult.path.lastIndexOf("/"))
    : "";

  const editArgs = args ? parseEditArgs(args) : null;
  const hasDiff = !!(editArgs?.oldString && editArgs?.newString && editArgs.oldString !== editArgs.newString);

  return (
    <div>
      {/* File info row — clickable to toggle diff */}
      <button
        type="button"
        onClick={() => hasDiff && setDiffOpen(!diffOpen)}
        className="flex w-full items-center justify-between py-1.5 text-left transition-colors"
        style={{ cursor: hasDiff ? "pointer" : "default" }}
      >
        <div className="flex min-w-0 items-center gap-2">
          <span className="h-[6px] w-[6px] shrink-0 rounded-full" style={{ background: "var(--orange, #ED8936)" }} />
          <span
            className="text-[12px] font-medium truncate"
            style={{ fontFamily: "var(--font-mono)", color: "var(--fill-primary)" }}
          >
            {fileName}
          </span>
          {dir && (
            <span className="text-[10px] truncate" style={{ color: "var(--fill-quaternary)" }}>
              {dir}
            </span>
          )}
        </div>
        <div className="flex items-center gap-2">
          {editResult.replacements > 1 && (
            <span className="text-[10px]" style={{ color: "var(--fill-quaternary)" }}>
              {t("diff_replacements", { count: editResult.replacements })}
            </span>
          )}
          <DiffStatBadge added={editResult.linesAdded} removed={editResult.linesRemoved} />
          {hasDiff && (
            <CaretRight
              size={12}
              className="shrink-0 transition-transform duration-150"
              style={{
                color: "var(--fill-quaternary)",
                transform: diffOpen ? "rotate(90deg)" : "rotate(0)",
              }}
            />
          )}
        </div>
      </button>

      {/* Expandable inline diff */}
      {diffOpen && hasDiff && (
        <div className="pb-1">
          <InlineDiff oldStr={editArgs!.oldString!} newStr={editArgs!.newString!} />
        </div>
      )}
    </div>
  );
}

export function isEditResult(toolName: string, result: string): boolean {
  if (toolName !== "edit_file") return false;
  return parseEditResult(result) !== null;
}
