import { useState, useEffect, useRef, useCallback, useMemo, memo, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
import {
  FileText, PenLine, Search, Terminal, Globe, Download, Monitor,
  Brain, Database, Image, Volume2, PackageSearch, PackagePlus,
  TableProperties, Play, Wrench, Check, X as XIcon, ChevronRight, Plug,
  Copy, Maximize2, ListTodo, Code2, Compass, ExternalLink,
} from "lucide-react";
import { useWorkspaceTabs } from "../shell/workspace-tabs";
import { TodoCard, isTodoResult } from "./TodoCard";
import { DiffCard, isEditResult } from "./DiffCard";
import { PlanApprovalCard, isPlanExitResult, type PlanApprovalMetadata } from "./PlanApprovalCard";
import { ICON } from "../../lib/ui-tokens";

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

export type ToolCategory = "shell" | "read" | "write" | "edit" | "search" | "web" | "mcp" | "default";

const CATEGORY_MAP: Record<string, ToolCategory> = {
  shell: "shell", shell_exec: "shell", code_execute: "shell",
  file_read: "read", read_file: "read", read_skill: "read",
  list_skills: "read", list_directory: "read",
  file_write: "write", write_file: "write", write_skill: "write",
  edit_file: "edit",
  file_search: "search", hub_search: "search", memory_search: "search",
  web_search: "web", web_fetch: "web", http_fetch: "web",
};

export function getToolCategory(name: string): ToolCategory {
  if (CATEGORY_MAP[name]) return CATEGORY_MAP[name];
  if (name.startsWith("mcp_")) return "mcp";
  return "default";
}

export function buildToolMeta(t: TFunction<"chat">): Record<string, { icon: ReactNode; label?: string }> {
  return {
    file_read: { icon: <FileText {...ICON.sm} />, label: t("tool_file_read") },
    file_write: { icon: <PenLine {...ICON.sm} />, label: t("tool_file_write") },
    file_search: { icon: <Search {...ICON.sm} />, label: t("tool_file_search") },
    shell: { icon: <Terminal {...ICON.sm} />, label: t("tool_shell") },
    shell_exec: { icon: <Terminal {...ICON.sm} />, label: t("tool_shell_exec") },
    web_search: { icon: <Globe {...ICON.sm} />, label: t("tool_web_search") },
    web_fetch: { icon: <Download {...ICON.sm} />, label: t("tool_web_fetch") },
    browser: { icon: <Monitor {...ICON.sm} />, label: t("tool_browser") },
    memory_search: { icon: <Brain {...ICON.sm} />, label: t("tool_memory_search") },
    memory_store: { icon: <Database {...ICON.sm} />, label: t("tool_memory_store") },
    image_generate: { icon: <Image {...ICON.sm} />, label: t("tool_image_generate") },
    text_to_speech: { icon: <Volume2 {...ICON.sm} />, label: t("tool_text_to_speech") },
    hub_search: { icon: <PackageSearch {...ICON.sm} />, label: t("tool_hub_search") },
    hub_install: { icon: <PackagePlus {...ICON.sm} />, label: t("tool_hub_install") },
    sql_query: { icon: <TableProperties {...ICON.sm} />, label: t("tool_sql_query") },
    code_execute: { icon: <Play {...ICON.sm} />, label: t("tool_code_execute") },
    read_file: { icon: <FileText {...ICON.sm} />, label: t("tool_read_file") },
    write_file: { icon: <PenLine {...ICON.sm} />, label: t("tool_write_file") },
    list_directory: { icon: <Search {...ICON.sm} />, label: t("tool_list_directory") },
    read_skill: { icon: <FileText {...ICON.sm} />, label: t("tool_read_skill") },
    list_skills: { icon: <Search {...ICON.sm} />, label: t("tool_list_skills") },
    write_skill: { icon: <PenLine {...ICON.sm} />, label: t("tool_write_skill") },
    http_fetch: { icon: <Globe {...ICON.sm} />, label: t("tool_http_fetch") },
    calculator: { icon: <TableProperties {...ICON.sm} />, label: t("tool_calculator") },
    todo_write: { icon: <ListTodo {...ICON.sm} />, label: t("tool_todo_write") },
    edit_file: { icon: <Code2 {...ICON.sm} />, label: t("tool_edit_file") },
    lsp: { icon: <Code2 {...ICON.sm} />, label: t("tool_lsp") },
    enter_plan_mode: { icon: <Compass {...ICON.sm} />, label: t("tool_enter_plan_mode") },
    exit_plan_mode: { icon: <Code2 {...ICON.sm} />, label: t("tool_exit_plan_mode") },
  };
}

const DEFAULT_META = { icon: <Wrench {...ICON.sm} /> };

function getMcpMeta(name: string): { icon: ReactNode; label: string } | null {
  if (!name.startsWith("mcp_")) return null;
  const rest = name.slice(4);
  const idx = rest.indexOf("_");
  const serverId = idx >= 0 ? rest.slice(0, idx) : rest;
  const toolName = idx >= 0 ? rest.slice(idx + 1) : "";
  return { icon: <Plug {...ICON.sm} />, label: `${serverId}/${toolName}` };
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

export function extractKeyInfo(tool: ToolCall): string | null {
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

const IMAGE_DATA_URI_RE = /!\[image\]\((data:image\/[^;]+;base64,[A-Za-z0-9+/=]+)\)/g;

function extractImages(text: string): { images: string[]; textOnly: string } {
  const images: string[] = [];
  const textOnly = text.replace(IMAGE_DATA_URI_RE, (_match, dataUri: string) => {
    images.push(dataUri);
    return "";
  }).trim();
  return { images, textOnly };
}

export function ImageViewer({ src }: { src: string }) {
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
        <div className="absolute right-2 top-2 flex gap-1 opacity-0 transition-opacity duration-150 group-hover:opacity-100">
          <button
            onClick={handleCopy}
            className="flex h-7 w-7 items-center justify-center rounded-md backdrop-blur-sm transition-colors hover:brightness-125"
            style={{ background: "rgba(0,0,0,0.55)", color: copied ? "var(--green)" : "#fff" }}
            title={copied ? t("copied") : t("copyImage")}
            aria-label={copied ? t("copied") : t("copyImage")}
          >
            {copied ? <Check {...ICON.sm} /> : <Copy {...ICON.sm} />}
          </button>
          <button
            onClick={(e) => { e.stopPropagation(); setLightbox(true); }}
            className="flex h-7 w-7 items-center justify-center rounded-md backdrop-blur-sm transition-colors hover:brightness-125"
            style={{ background: "rgba(0,0,0,0.55)", color: "#fff" }}
            title={t("viewFullImage")}
            aria-label={t("viewFullImage")}
          >
            <Maximize2 {...ICON.sm} />
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
              {copied ? <Check {...ICON.sm} /> : <Copy {...ICON.sm} />}
              {copied ? t("copied") : t("copy", { ns: "common" })}
            </button>
            <button
              onClick={(e) => { e.stopPropagation(); setLightbox(false); }}
              className="flex h-9 w-9 items-center justify-center rounded-lg transition-colors hover:brightness-125"
              style={{ background: "rgba(255,255,255,0.15)", color: "#fff" }}
              aria-label={t("closePreview")}
            >
              <XIcon {...ICON.md} />
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
      className="ml-6 mt-0.5 mb-1 overflow-hidden rounded-md"
      style={{ border: "0.5px solid var(--separator)", background: "var(--bg-primary)" }}
    >
      <div className="flex items-center gap-2 px-2.5 py-1" style={{ borderBottom: "0.5px solid var(--separator)" }}>
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
    <div className="mt-1 space-y-2">
      {images.map((src, i) => (
        <ImageViewer key={i} src={src} />
      ))}
      {formatted && (
        <>
          <pre
            className="overflow-x-auto whitespace-pre-wrap break-all rounded-md p-2 text-[11px] leading-[1.55]"
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
              className="mt-0.5 cursor-pointer text-[11px] font-medium"
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

function ShellResultSummary({ result }: { result: string }) {
  const { t } = useTranslation("chat");
  const setActiveTab = useWorkspaceTabs((s) => s.setActiveTab);

  const parsed = useMemo(() => {
    const exitMatch = result.match(/exit_code=(\d+)/);
    const durationMatch = result.match(/duration_ms=(\d+)/);
    const cwdMatch = result.match(/cwd=(.+)/);
    return {
      exitCode: exitMatch ? Number(exitMatch[1]) : null,
      durationMs: durationMatch ? Number(durationMatch[1]) : null,
      cwd: cwdMatch ? cwdMatch[1].trim() : null,
    };
  }, [result]);

  return (
    <div
      className="ml-6 flex items-center gap-3 px-2.5 pb-1.5 text-[10px]"
      style={{ color: "var(--fill-quaternary)" }}
    >
      {parsed.exitCode !== null && (
        <span style={{ color: parsed.exitCode === 0 ? "var(--green)" : "var(--red)" }}>
          exit_code={parsed.exitCode}
        </span>
      )}
      {parsed.durationMs !== null && (
        <span>{t("duration", { ns: "common", defaultValue: "duration" })}={formatDuration(parsed.durationMs)}</span>
      )}
      {parsed.cwd && parsed.cwd !== "." && (
        <span className="truncate max-w-[120px]" title={parsed.cwd}>cwd={parsed.cwd}</span>
      )}
      <button
        onClick={() => setActiveTab("terminal")}
        className="ml-auto flex items-center gap-0.5 text-[10px] font-medium"
        style={{ color: "var(--tint)", cursor: "pointer" }}
      >
        <ExternalLink size={10} strokeWidth={1.5} />
        {t("viewTerminal", { ns: "chat", defaultValue: "Terminal" })}
      </button>
    </div>
  );
}

/**
 * Card-style step indicator with category-colored icon badge.
 * 36px row height, 1px border, 8px radius.
 */
export const StepIndicator = memo(function StepIndicator({ tool, compact }: { tool: ToolCall; compact?: boolean }) {
  const { t } = useTranslation("chat");
  const toolMeta = useMemo(() => buildToolMeta(t), [t]);
  const [expanded, setExpanded] = useState(false);
  const mcpMeta = getMcpMeta(tool.name);
  const meta = mcpMeta ?? toolMeta[tool.name] ?? DEFAULT_META;
  const label = meta.label ?? tool.name;
  const keyInfo = useMemo(() => extractKeyInfo(tool), [tool.args]);
  const hasDetails = !!(tool.args || tool.result);
  const category = getToolCategory(tool.name);
  const isShell = category === "shell";

  const isRunning = tool.status === "running";
  const isError = tool.status === "error";

  const resultImages = tool.result ? extractImages(tool.result).images : [];
  const hasSpecialResult = tool.result && (
    isTodoResult(tool.name, tool.result) ||
    isEditResult(tool.name, tool.result) ||
    isPlanExitResult(tool.name, tool.result, tool.metadata as PlanApprovalMetadata | undefined)
  );
  const canExpand = hasDetails && !hasSpecialResult && !isShell;

  return (
    <div
      className={`step-indicator${expanded ? " open" : ""}`}
      style={{
        border: compact ? "none" : "1px solid var(--step-border)",
        borderRadius: compact ? "0" : "var(--step-radius)",
        marginBottom: "var(--step-gap)",
        overflow: "hidden",
      }}
    >
      {/* Header row */}
      <button
        onClick={() => canExpand && setExpanded(!expanded)}
        className="tc-h flex w-full items-center gap-2 px-2.5 text-left transition-colors duration-100"
        style={{
          cursor: canExpand ? "pointer" : "default",
          minHeight: "var(--step-height)",
          background: isRunning ? "color-mix(in srgb, var(--tint) 4%, transparent)" : undefined,
        }}
        onMouseEnter={(e) => { if (!isRunning) (e.currentTarget as HTMLElement).style.background = "var(--step-hover-bg)"; }}
        onMouseLeave={(e) => { if (!isRunning) (e.currentTarget as HTMLElement).style.background = ""; }}
        aria-expanded={canExpand ? expanded : undefined}
      >
        {/* Category-colored icon badge */}
        <span
          className="tico grid shrink-0 place-items-center"
          style={{
            width: "var(--step-icon-size)",
            height: "var(--step-icon-size)",
            borderRadius: "var(--step-icon-radius)",
            background: `var(--tc-${category}-bg)`,
            color: `var(--tc-${category}-fg)`,
          } as React.CSSProperties}
        >
          {meta.icon}
        </span>

        {/* Label + key info */}
        <span className="flex min-w-0 flex-1 items-center gap-1.5 text-[12px]">
          <span className="tl shrink-0 font-medium" style={{ color: isError ? "var(--red)" : "var(--fill-secondary)" }}>
            {label}
          </span>
          {keyInfo && (
            <span
              className="tp min-w-0 truncate text-[11px]"
              style={{ color: "var(--fill-quaternary)", fontFamily: "var(--font-mono)" }}
              title={keyInfo}
            >
              {keyInfo}
            </span>
          )}
        </span>

        {/* Status area: dot + duration */}
        <span className="ts flex shrink-0 items-center gap-1.5">
          {/* Status dot */}
          <span className="flex h-[14px] w-[14px] items-center justify-center">
            {isRunning ? (
              <span
                className="sd inline-block h-[5px] w-[5px] rounded-full border-[1px]"
                style={{
                  borderColor: "var(--tint) transparent transparent transparent",
                  animation: "spin 0.8s linear infinite",
                }}
              />
            ) : isError ? (
              <span className="sd inline-block h-[5px] w-[5px] rounded-full" style={{ background: "var(--red)" }} />
            ) : (
              <span className="sd inline-block h-[5px] w-[5px] rounded-full" style={{ background: "var(--green)" }} />
            )}
          </span>

          {/* Duration */}
          <span className="text-[10px] tabular-nums" style={{ color: "var(--fill-quaternary)" }}>
            {isRunning && tool.startTime ? <ElapsedTimer startTime={tool.startTime} /> : null}
            {!isRunning && tool.duration ? formatDuration(tool.duration) : null}
          </span>
        </span>

        {/* Expand chevron */}
        {canExpand && (
          <ChevronRight
            size={12}
            strokeWidth={1.5}
            className="tv shrink-0 transition-transform duration-150"
            style={{
              color: "var(--fill-quaternary)",
              transform: expanded ? "rotate(90deg)" : "rotate(0)",
            }}
          />
        )}
      </button>

      {/* Auto-display images from tool results */}
      {resultImages.length > 0 && (
        <div className="px-2.5 pb-2 space-y-1.5">
          {resultImages.map((src, i) => (
            <ImageViewer key={i} src={src} />
          ))}
        </div>
      )}

      {/* Streaming diff preview while edit_file is running */}
      {isRunning && isEditLikeTool(tool.name) && tool.args && (
        <StreamingDiffPreview args={tool.args} />
      )}

      {/* Specialized results — rendered as independent blocks below the step row */}
      {!isRunning && tool.result && isTodoResult(tool.name, tool.result) && (
        <div className="px-2.5 pb-2">
          <TodoCard result={tool.result} />
        </div>
      )}
      {!isRunning && tool.result && isEditResult(tool.name, tool.result) && (
        <div className="px-2.5 pb-2">
          <DiffCard result={tool.result} args={tool.args} />
        </div>
      )}
      {!isRunning && tool.result && isPlanExitResult(tool.name, tool.result, tool.metadata as PlanApprovalMetadata | undefined) && (
        <div className="px-2.5 pb-2">
          <PlanApprovalCard result={tool.result} metadata={tool.metadata as PlanApprovalMetadata | undefined} />
        </div>
      )}

      {/* Shell tools: simplified inline status + "View in Terminal" */}
      {isShell && !isRunning && tool.result && (
        <ShellResultSummary result={tool.result} />
      )}

      {/* Expandable body — grid-template-rows animation (hidden for shell tools) */}
      {category !== "shell" && (
        <div
          className="tc-bd"
          style={{
            display: "grid",
            gridTemplateRows: expanded && hasDetails && !hasSpecialResult ? "1fr" : "0fr",
            transition: "grid-template-rows 260ms cubic-bezier(0.23, 1, 0.32, 1)",
          }}
        >
          <div className="tc-bd-in overflow-hidden">
            {hasDetails && !hasSpecialResult && (
              <div
                className="px-2.5 pb-2 pt-1.5"
                style={{ borderTop: "1px solid var(--separator)" }}
              >
                {tool.args && (
                  <div className="mb-1.5">
                    <span className="text-[10px] font-semibold uppercase tracking-wider" style={{ color: "var(--fill-quaternary)" }}>{t("params")}</span>
                    <pre
                      className="mt-0.5 overflow-x-auto whitespace-pre-wrap break-all rounded-md p-2 text-[11px] leading-[1.5]"
                      style={{
                        background: "var(--bg-primary)",
                        color: "var(--fill-secondary)",
                        border: "0.5px solid var(--separator)",
                        fontFamily: "var(--font-mono)",
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
        </div>
      )}
    </div>
  );
});
