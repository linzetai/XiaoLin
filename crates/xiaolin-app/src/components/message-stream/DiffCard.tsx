/**
 * Specialized renderer for edit_file tool results.
 * Shows a compact diff summary with file path and line change stats.
 */

import { FileCode, Plus, Minus } from "lucide-react";
import { ICON } from "../../lib/ui-tokens";

interface EditResult {
  edited: boolean;
  path: string;
  replacements: number;
  bytes: number;
  diffStat: string;
  linesAdded: number;
  linesRemoved: number;
}

function parseEditResult(result: string): EditResult | null {
  try {
    const parsed = JSON.parse(result);
    if (parsed.edited && parsed.path && typeof parsed.linesAdded === "number") {
      return parsed as EditResult;
    }
  } catch { /* not JSON edit result */ }
  return null;
}

function DiffStatBadge({ added, removed }: { added: number; removed: number }) {
  const total = added + removed;
  if (total === 0) return <span className="text-[10px]" style={{ color: "var(--fill-quaternary)" }}>no change</span>;

  const maxDots = 5;
  const addDots = total > 0 ? Math.max(1, Math.round((added / total) * maxDots)) : 0;
  const remDots = total > 0 ? Math.max(total > 0 && removed > 0 ? 1 : 0, maxDots - addDots) : 0;

  return (
    <div className="flex items-center gap-1.5">
      {added > 0 && (
        <span className="flex items-center gap-0.5 text-[10px] font-medium tabular-nums" style={{ color: "var(--green, #48BB78)" }}>
          <Plus size={14} strokeWidth={2.5} />
          {added}
        </span>
      )}
      {removed > 0 && (
        <span className="flex items-center gap-0.5 text-[10px] font-medium tabular-nums" style={{ color: "var(--red, #FC8181)" }}>
          <Minus size={14} strokeWidth={2.5} />
          {removed}
        </span>
      )}
      <div className="flex gap-[2px]">
        {Array.from({ length: addDots }).map((_, i) => (
          <div
            key={`a${i}`}
            className="h-[6px] w-[6px] rounded-[1px]"
            style={{ background: "var(--green, #48BB78)" }}
          />
        ))}
        {Array.from({ length: remDots }).map((_, i) => (
          <div
            key={`r${i}`}
            className="h-[6px] w-[6px] rounded-[1px]"
            style={{ background: "var(--red, #FC8181)" }}
          />
        ))}
        {Array.from({ length: Math.max(0, maxDots - addDots - remDots) }).map((_, i) => (
          <div
            key={`n${i}`}
            className="h-[6px] w-[6px] rounded-[1px]"
            style={{ background: "var(--fill-quaternary)" }}
          />
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

function InlineDiff({ oldStr, newStr }: { oldStr: string; newStr: string }) {
  const oldLines = oldStr.split("\n");
  const newLines = newStr.split("\n");
  const maxLines = 12;

  const allLines: Array<{ type: "remove" | "add" | "context"; text: string }> = [];

  let oi = 0;
  let ni = 0;
  while (oi < oldLines.length || ni < newLines.length) {
    if (oi < oldLines.length && ni < newLines.length && oldLines[oi] === newLines[ni]) {
      allLines.push({ type: "context", text: oldLines[oi] });
      oi++;
      ni++;
    } else if (oi < oldLines.length) {
      allLines.push({ type: "remove", text: oldLines[oi] });
      oi++;
    } else if (ni < newLines.length) {
      allLines.push({ type: "add", text: newLines[ni] });
      ni++;
    }
  }

  const truncated = allLines.length > maxLines;
  const display = truncated ? allLines.slice(0, maxLines) : allLines;

  return (
    <pre
      className="mt-1.5 overflow-x-auto rounded-md text-[11px] leading-[1.6]"
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
        <div className="px-2 py-0.5" style={{ color: "var(--fill-quaternary)" }}>
          ... {allLines.length - maxLines} more lines
        </div>
      )}
    </pre>
  );
}

export function DiffCard({ result, args }: { result: string; args?: string }) {
  const editResult = parseEditResult(result);
  if (!editResult) return null;

  const fileName = editResult.path.split("/").pop() || editResult.path;
  const dir = editResult.path.includes("/")
    ? editResult.path.slice(0, editResult.path.lastIndexOf("/"))
    : "";

  const editArgs = args ? parseEditArgs(args) : null;
  const showDiff = editArgs?.oldString && editArgs?.newString && editArgs.oldString !== editArgs.newString;

  return (
    <div
      className="mt-1.5 overflow-hidden rounded-lg"
      style={{
        border: "0.5px solid var(--separator)",
        background: "var(--bg-secondary)",
      }}
    >
      <div className="flex items-center justify-between px-3 py-2">
        <div className="flex min-w-0 items-center gap-2">
          <FileCode {...ICON.md} style={{ color: "var(--tint, #4299E1)" }} />
          <div className="min-w-0">
            <span className="text-[12px] font-medium" style={{ color: "var(--fill-primary)" }}>
              {fileName}
            </span>
            {dir && (
              <span className="ml-1.5 text-[10px]" style={{ color: "var(--fill-quaternary)" }}>
                {dir}
              </span>
            )}
          </div>
        </div>
        <div className="flex items-center gap-3">
          {editResult.replacements > 1 && (
            <span className="text-[10px]" style={{ color: "var(--fill-quaternary)" }}>
              {editResult.replacements} replacements
            </span>
          )}
          <DiffStatBadge added={editResult.linesAdded} removed={editResult.linesRemoved} />
        </div>
      </div>

      {showDiff && (
        <div
          className="px-3 pb-2"
          style={{ borderTop: "0.5px solid var(--separator)" }}
        >
          <InlineDiff oldStr={editArgs!.oldString!} newStr={editArgs!.newString!} />
        </div>
      )}
    </div>
  );
}

export function isEditResult(toolName: string, result: string): boolean {
  if (toolName !== "edit_file") return false;
  try {
    const parsed = JSON.parse(result);
    return parsed.edited === true && typeof parsed.path === "string";
  } catch { return false; }
}
