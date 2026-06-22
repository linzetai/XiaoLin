/**
 * @deprecated Use StepIndicator from ./StepIndicator.tsx instead.
 * Kept temporarily for backward compatibility during message-stream-redesign transition.
 */
import { useState, useEffect, useRef, useCallback, useMemo, memo, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import {
  Wrench, Check, X as XIcon, CaretRight, Plug,
  Copy, ArrowsOut,
} from "@phosphor-icons/react";
import { TodoCard, isTodoResult } from "./TodoCard";
import { DiffCard, isEditResult } from "./DiffCard";
import { PlanApprovalCard, isPlanExitResult, type PlanApprovalMetadata } from "./PlanApprovalCard";
import { buildToolMeta } from "./StepIndicator";
import { ICON_SIZE } from "../../lib/ui-tokens";
import { parseMcpToolName } from "../../lib/mcpNaming";

export interface ToolCall {
  id: string;
  name: string;
  status: "running" | "success" | "error";
  args?: string;
  result?: string;
  duration?: number;
  startTime?: number;
  metadata?: Record<string, unknown> | null;
}

const DEFAULT_META = { icon: <Wrench  /> };

function getMcpMeta(name: string): { icon: ReactNode; label: string } | null {
  const parsed = parseMcpToolName(name);
  if (!parsed) return null;
  return { icon: <Plug  />, label: `${parsed.serverId}/${parsed.toolName}` };
}


function ElapsedTimer({ startTime }: { startTime: number }) {
  const [elapsed, setElapsed] = useState(0);
  const ref = useRef<ReturnType<typeof setInterval>>(null);

  useEffect(() => {
    ref.current = setInterval(() => setElapsed(Date.now() - startTime), 200);
    return () => { if (ref.current !== null) clearInterval(ref.current); };
  }, [startTime]);

  return <span>{(elapsed / 1000).toFixed(1)}s</span>;
}

function formatDuration(ms: number): string {
  if (ms < 1000) return `${ms}ms`;
  return `${(ms / 1000).toFixed(1)}s`;
}

function extractKeyInfo(tool: ToolCall): string | null {
  if (!tool.args) return null;
  try {
    const args = JSON.parse(tool.args);
    if (args.path || args.file) return args.path ?? args.file;
    if (args.command || args.cmd) return args.command ?? args.cmd;
    if (args.query) return args.query;
    if (args.url) return args.url;
    if (args.directory || args.dir) return args.directory ?? args.dir;
    const keys = Object.keys(args);
    if (keys.length === 1) {
      const val = args[keys[0]];
      if (typeof val === "string") return val.slice(0, 120);
      if (typeof val === "number" || typeof val === "boolean") return String(val);
    }
  } catch {
    return tool.args.length < 120 ? tool.args : null;
  }
  return null;
}

function tryPrettyJson(text: string): string {
  try {
    const parsed = JSON.parse(text);
    return JSON.stringify(parsed, null, 2);
  } catch {
    return text;
  }
}

function ImageViewer({ src }: { src: string }) {
  const { t } = useTranslation("chat");
  const [lightbox, setLightbox] = useState(false);
  const [copied, setCopied] = useState(false);

  const handleCopy = useCallback(async (e: React.MouseEvent) => {
    e.stopPropagation();
    try {
      const resp = await fetch(src);
      const blob = await resp.blob();
      await navigator.clipboard.write([new ClipboardItem({ [blob.type]: blob })]);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    } catch { /* clipboard API may not be available */ }
  }, [src]);

  return (
    <>
      <div className="group relative overflow-hidden rounded-md" style={{ border: "0.5px solid var(--separator)" }}>
        <img
          src={src}
          alt="Tool output"
          className="block max-h-[400px] w-full cursor-pointer object-contain"
          style={{ background: "var(--bg-primary)" }}
          onClick={() => setLightbox(true)}
        />
        <div
          className="absolute right-2 top-2 flex gap-1 opacity-0 transition-opacity duration-150 group-hover:opacity-100"
        >
          <button
            onClick={handleCopy}
            className="flex h-7 w-7 items-center justify-center rounded-md backdrop-blur-sm transition-colors hover:brightness-125"
            style={{ background: "rgba(0,0,0,0.55)", color: copied ? "var(--green)" : "#fff" }}
            title={copied ? t("copied") : t("copyImage")}
            aria-label={copied ? t("copied") : t("copyImage")}
          >
            {copied ? <Check  /> : <Copy  />}
          </button>
          <button
            onClick={(e) => { e.stopPropagation(); setLightbox(true); }}
            className="flex h-7 w-7 items-center justify-center rounded-md backdrop-blur-sm transition-colors hover:brightness-125"
            style={{ background: "rgba(0,0,0,0.55)", color: "#fff" }}
            title={t("viewFullImage")}
            aria-label={t("viewFullImage")}
          >
            <ArrowsOut />
          </button>
        </div>
      </div>

      {lightbox && (
        <div
          className="fixed inset-0 z-[9999] flex items-center justify-center"
          style={{ background: "rgba(0,0,0,0.85)" }}
          onClick={() => setLightbox(false)}
          onKeyDown={(e) => { if (e.key === "Escape") setLightbox(false); }}
          role="dialog"
          aria-modal="true"
          aria-label={t("imagePreview")}
          tabIndex={-1}
          ref={(el) => el?.focus()}
        >
          <div className="absolute right-4 top-4 flex gap-2">
            <button
              onClick={handleCopy}
              className="flex h-9 items-center gap-1.5 rounded-lg px-3 text-[12px] font-medium transition-colors hover:brightness-125"
              style={{ background: "rgba(255,255,255,0.15)", color: copied ? "var(--green)" : "#fff" }}
              aria-label={copied ? t("copied") : t("copyImage")}
            >
              {copied ? <Check  /> : <Copy  />}
              {copied ? t("copied") : t("copy", { ns: "common" })}
            </button>
            <button
              onClick={(e) => { e.stopPropagation(); setLightbox(false); }}
              className="flex h-9 w-9 items-center justify-center rounded-lg transition-colors hover:brightness-125"
              style={{ background: "rgba(255,255,255,0.15)", color: "#fff" }}
              aria-label={t("closePreview")}
            >
              <XIcon size={ICON_SIZE.md} />
            </button>
          </div>
          <img
            src={src}
            alt="Tool output (full)"
            className="max-h-[90vh] max-w-[90vw] rounded-lg object-contain"
            onClick={(e) => e.stopPropagation()}
          />
        </div>
      )}
    </>
  );
}

function isEditLikeTool(name: string): boolean {
  return name === "edit_file" || name === "write_file" || name === "file_write";
}

function StreamingDiffPreview({ args }: { args: string }) {
  const { t } = useTranslation("chat");
  const parsed = useMemo(() => {
    try {
      const a = JSON.parse(args);
      const oldStr = a.old_string ?? a.oldString ?? "";
      const newStr = a.new_string ?? a.newString ?? a.content ?? "";
      const filePath = a.file_path ?? a.path ?? a.file ?? "";
      if (!newStr) return null;
      return { oldStr, newStr, filePath };
    } catch { return null; }
  }, [args]);

  if (!parsed) return null;
  const { oldStr, newStr, filePath } = parsed;
  const fileName = filePath.split("/").pop() || filePath;
  const isCreate = !oldStr;

  return (
    <div
      className="mx-3 mb-2 overflow-hidden rounded-md"
      style={{ border: "0.5px solid var(--separator)", background: "var(--bg-primary)" }}
    >
      <div className="flex items-center gap-2 px-2.5 py-1.5" style={{ borderBottom: "0.5px solid var(--separator)" }}>
        <span className="text-[10px] font-medium uppercase tracking-wider" style={{ color: "var(--tint, #4299E1)" }}>
          {isCreate ? t("create") : t("changePreview")}
        </span>
        {fileName && (
          <span className="truncate text-[10px]" style={{ color: "var(--fill-quaternary)" }}>{fileName}</span>
        )}
      </div>
      <pre
        className="overflow-x-auto text-[11px] leading-[1.6]"
        style={{ fontFamily: 'var(--font-mono)', maxHeight: "200px", overflowY: "auto" }}
      >
        {isCreate ? (
          newStr.split("\n").slice(0, 20).map((line: string, i: number) => (
            <div key={i} className="px-2" style={{ background: "color-mix(in srgb, var(--green, #48BB78) 8%, transparent)", color: "var(--green, #48BB78)" }}>
              <span className="mr-2 inline-block w-3 select-none text-right opacity-50">+</span>
              {line || " "}
            </div>
          ))
        ) : (
          renderSimpleDiff(oldStr, newStr)
        )}
        {(isCreate ? newStr : oldStr + newStr).split("\n").length > 20 && (
          <div className="px-2 py-0.5" style={{ color: "var(--fill-quaternary)" }}>...</div>
        )}
      </pre>
    </div>
  );
}

function renderSimpleDiff(oldStr: string, newStr: string): ReactNode[] {
  const oldLines = oldStr.split("\n");
  const newLines = newStr.split("\n");
  const result: ReactNode[] = [];
  const maxLines = 20;
  let count = 0;

  let oi = 0;
  let ni = 0;
  while ((oi < oldLines.length || ni < newLines.length) && count < maxLines) {
    if (oi < oldLines.length && ni < newLines.length && oldLines[oi] === newLines[ni]) {
      result.push(
        <div key={`c${count}`} className="px-2" style={{ color: "var(--fill-tertiary)" }}>
          <span className="mr-2 inline-block w-3 select-none text-right opacity-50"> </span>
          {oldLines[oi] || " "}
        </div>
      );
      oi++; ni++; count++;
    } else if (oi < oldLines.length) {
      result.push(
        <div key={`r${count}`} className="px-2" style={{ background: "color-mix(in srgb, var(--red, #FC8181) 8%, transparent)", color: "var(--red, #FC8181)" }}>
          <span className="mr-2 inline-block w-3 select-none text-right opacity-50">-</span>
          {oldLines[oi] || " "}
        </div>
      );
      oi++; count++;
    } else {
      result.push(
        <div key={`a${count}`} className="px-2" style={{ background: "color-mix(in srgb, var(--green, #48BB78) 8%, transparent)", color: "var(--green, #48BB78)" }}>
          <span className="mr-2 inline-block w-3 select-none text-right opacity-50">+</span>
          {newLines[ni] || " "}
        </div>
      );
      ni++; count++;
    }
  }
  return result;
}

const MAX_OUTPUT_LINES = 16;
const MAX_OUTPUT_CHARS = 1200;

const IMAGE_DATA_URI_RE = /!\[image\]\((data:image\/[^;]+;base64,[A-Za-z0-9+/=]+)\)/g;

function extractImages(text: string): { images: string[]; textOnly: string } {
  const images: string[] = [];
  const textOnly = text.replace(IMAGE_DATA_URI_RE, (_match, dataUri: string) => {
    images.push(dataUri);
    return "";
  }).trim();
  return { images, textOnly };
}

function OutputBlock({ content, error }: { content: string; error?: boolean }) {
  const { t } = useTranslation("chat");
  const [expanded, setExpanded] = useState(false);
  const { images, textOnly } = extractImages(content);
  const formatted = textOnly ? tryPrettyJson(textOnly) : "";
  const lines = formatted.split("\n");
  const needsTruncate = lines.length > MAX_OUTPUT_LINES || formatted.length > MAX_OUTPUT_CHARS;
  const display = expanded
    ? formatted
    : lines.slice(0, MAX_OUTPUT_LINES).join("\n").slice(0, MAX_OUTPUT_CHARS);

  return (
    <div className="mt-1.5 space-y-2">
      {images.map((src, i) => (
        <ImageViewer key={i} src={src} />
      ))}
      {formatted && (
        <>
          <pre
            className="overflow-x-auto whitespace-pre-wrap break-all rounded-md p-2.5 text-[11px] leading-[1.55]"
            style={{
              background: "var(--bg-primary)",
              color: error ? "var(--red)" : "var(--fill-secondary)",
              border: `0.5px solid var(--separator)`,
              fontFamily: 'var(--font-mono)',
              maxHeight: expanded ? "none" : "280px",
              overflowY: expanded ? "visible" : "hidden",
            }}
          >
            {display}
            {!expanded && needsTruncate && <span style={{ color: "var(--fill-quaternary)" }}>…</span>}
          </pre>
          {needsTruncate && (
            <button
              onClick={() => setExpanded(!expanded)}
              className="mt-1 cursor-pointer text-[11px] font-medium"
              style={{ color: "var(--fill-tertiary)" }}
            >
              {expanded ? t("collapse", { ns: "common" }) : t("expandAllLines", { count: lines.length })}
            </button>
          )}
        </>
      )}
    </div>
  );
}

export const ToolCallCard = memo(function ToolCallCard({ tool }: { tool: ToolCall }) {
  const { t } = useTranslation("chat");
  const toolMeta = useMemo(() => buildToolMeta(t), [t]);
  const [expanded, setExpanded] = useState(false);
  const mcpMeta = getMcpMeta(tool.name);
  const meta = mcpMeta ?? toolMeta[tool.name] ?? DEFAULT_META;
  const label = meta.label ?? tool.name;
  const keyInfo = useMemo(() => extractKeyInfo(tool), [tool.args]);
  const hasDetails = !!(tool.args || tool.result);

  const isRunning = tool.status === "running";
  const isError = tool.status === "error";

  const resultImages = tool.result ? extractImages(tool.result).images : [];

  return (
    <div
      className="my-1.5 overflow-hidden rounded-lg"
      style={{
        border: `0.5px solid ${isError ? "color-mix(in srgb, var(--red) 30%, transparent)" : "var(--border-subtle)"}`,
        borderLeft: isError ? "3px solid var(--red)" : "3px solid var(--tint)",
        background: isError ? "color-mix(in srgb, var(--red) 4%, transparent)" : "var(--bg-surface)",
        boxShadow: isError ? "none" : "inset 0 1px 0 var(--highlight-top)",
        animation: "slide-up var(--duration-fast) var(--ease-out)",
        maxWidth: "min(100%, var(--content-max-w, 600px))",
      }}
    >
      {/* Header — always visible, clickable to expand */}
      <button
        onClick={() => hasDetails && setExpanded(!expanded)}
        className="flex w-full items-center gap-2 px-3 py-1.5 text-left transition-colors duration-100"
        style={{ cursor: hasDetails ? "pointer" : "default" }}
        aria-expanded={hasDetails ? expanded : undefined}
      >
        {/* Status icon */}
        <span className="flex h-4 w-4 shrink-0 items-center justify-center">
          {isRunning ? (
            <span
              className="inline-block h-3 w-3 rounded-full border-[1.5px]"
              style={{
                borderColor: "var(--tint) transparent transparent transparent",
                animation: "spin 0.8s linear infinite",
              }}
            />
          ) : isError ? (
            <XIcon  style={{ color: "var(--red)", animation: "shake 0.3s ease-in-out" }} />
          ) : (
            <Check  style={{ color: "var(--green)", animation: "scale-spring var(--duration-normal) var(--ease-spring)" }} />
          )}
        </span>

        {/* Tool icon + label + key info */}
        <span className="flex min-w-0 flex-1 items-center gap-1.5 text-[12px]">
          <span className="shrink-0" style={{ color: "var(--fill-tertiary)" }}>{meta.icon}</span>
          <span className="shrink-0 font-medium" style={{ color: isError ? "var(--red)" : "var(--fill-primary)" }}>
            {label}
          </span>
          {keyInfo && (
            <span
              className="min-w-0 truncate font-mono text-[11px]"
              style={{ color: "var(--fill-tertiary)" }}
              title={keyInfo}
            >
              {keyInfo}
            </span>
          )}
        </span>

        {/* Duration */}
        <span className="shrink-0 text-[10px] tabular-nums" style={{ color: "var(--fill-quaternary)" }}>
          {isRunning && tool.startTime ? <ElapsedTimer startTime={tool.startTime} /> : null}
          {!isRunning && tool.duration ? formatDuration(tool.duration) : null}
        </span>

        {/* Expand chevron */}
        {hasDetails && (
          <CaretRight
            className="shrink-0 transition-transform duration-150"
            style={{
              color: "var(--fill-tertiary)",
              transform: expanded ? "rotate(90deg)" : "rotate(0)",
              transition: "transform var(--duration-normal) var(--ease-spring)",
            }}
          />
        )}
      </button>

      {isRunning && (
        <div className="h-[3px] w-full overflow-hidden" style={{ background: "var(--bg-tertiary)" }}>
          <div className="h-full w-1/3 rounded-full" style={{ background: "linear-gradient(90deg, transparent 0%, var(--tint) 50%, transparent 100%)", animation: "shimmer 1.5s ease-in-out infinite" }} />
        </div>
      )}

      {/* Auto-display images from tool results without needing to expand */}
      {resultImages.length > 0 && (
        <div className="px-3 pb-2 space-y-2">
          {resultImages.map((src, i) => (
            <ImageViewer key={i} src={src} />
          ))}
        </div>
      )}

      {/* Streaming diff preview — show while edit_file/write_file is running */}
      {isRunning && isEditLikeTool(tool.name) && tool.args && (
        <StreamingDiffPreview args={tool.args} />
      )}

      {/* Specialized tool result cards — shown without expanding */}
      {!isRunning && tool.result && isTodoResult(tool.name, tool.result) && (
        <div className="px-3 pb-2">
          <TodoCard result={tool.result} />
        </div>
      )}
      {!isRunning && tool.result && isEditResult(tool.name, tool.result) && (
        <div className="px-3 pb-2">
          <DiffCard result={tool.result} args={tool.args} />
        </div>
      )}
      {!isRunning && tool.result && isPlanExitResult(tool.name, tool.result, tool.metadata as PlanApprovalMetadata | undefined) && (
        <div className="px-3 pb-2">
          <PlanApprovalCard result={tool.result} metadata={tool.metadata as PlanApprovalMetadata | undefined} />
        </div>
      )}

      {/* Expanded details */}
      {expanded && hasDetails && (
        <div
          className="px-3 pb-2.5"
          style={{
            borderTop: `0.5px solid var(--separator)`,
            animation: "fade-slide-up var(--duration-normal) var(--ease-out)",
          }}
        >
          {tool.args && (
            <div className="mt-1.5">
              <span className="text-[10px] font-semibold uppercase tracking-wider" style={{ color: "var(--fill-quaternary)" }}>{t("params")}</span>
              <pre
                className="mt-1 overflow-x-auto whitespace-pre-wrap break-all rounded-md p-2 text-[11px] leading-[1.5]"
                style={{
                  background: "var(--bg-primary)",
                  color: "var(--fill-secondary)",
                  border: `0.5px solid var(--separator)`,
                  fontFamily: 'var(--font-mono)',
                  maxHeight: "200px",
                  overflowY: "auto",
                }}
              >
                {tryPrettyJson(tool.args)}
              </pre>
            </div>
          )}
          {tool.result && <OutputBlock content={tool.result} error={isError} />}
        </div>
      )}
    </div>
  );
});
