import { useState, useRef, useCallback, useEffect, useMemo } from "react";
import { Virtuoso, type VirtuosoHandle } from "react-virtuoso";
import { useAgentStore, type ChatMessage, type ChatUsage } from "../../lib/agent-store";
import { MarkdownContent } from "./MarkdownContent";
import { ToolCallCard, type ToolCall } from "./ToolCallCard";
import { MentionInput, type MentionInputHandle, type InlineMention, type MentionOption } from "./MentionInput";
import {
  Image as ImageIcon, FileText, Paperclip, File, Folder, Sparkles,
  X, ChevronUp, ChevronDown, Settings2, FolderOpen, ArrowUp,
  MessageSquare, Upload, Search, Square, Clock, ArrowUpRight, ArrowDownRight,
} from "lucide-react";
import * as api from "../../lib/api";
import * as transport from "../../lib/transport";
import { ClawIcon } from "../layout/ClawIcon";
import { open as tauriOpenDialog } from "@tauri-apps/plugin-dialog";

function ts(d: Date) {
  return d.toLocaleTimeString("zh-CN", { hour: "2-digit", minute: "2-digit" });
}

/* ─── Detached Stream Registry ─── */
interface DetachedStream {
  agentId: string;
  chatId: string;
  acc: string;
  toolCalls: ToolCall[];
  done: boolean;
  error: boolean;
  sessionId?: string;
  scrollPosition?: number;
  cleanup: () => void;
}

const detachedStreams = new Map<string, DetachedStream>();
const MAX_DETACHED_STREAMS = 64;

/* ─── Attached File Pill ─── */
interface AttachedFile {
  name: string;
  size: number;
  type: string;
  file: File;
  previewUrl?: string;
}

function formatSize(bytes: number) {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

function FilePill({ file, onRemove }: { file: AttachedFile; onRemove: () => void }) {
  const isImage = file.type.startsWith("image/");
  const icon = isImage
    ? <ImageIcon size={12} strokeWidth={1.5} />
    : file.type.includes("pdf")
      ? <FileText size={12} strokeWidth={1.5} />
      : <Paperclip size={12} strokeWidth={1.5} />;

  if (isImage && file.previewUrl) {
    return (
      <div
        className="relative inline-block rounded-lg overflow-hidden"
        style={{
          border: `0.5px solid var(--separator)`,
          animation: "pop 0.2s ease-out",
        }}
      >
        <img
          src={file.previewUrl}
          alt={file.name}
          className="block max-h-[80px] max-w-[120px] object-cover"
        />
        <button
          onClick={onRemove}
          className="absolute top-0.5 right-0.5 flex h-4 w-4 cursor-pointer items-center justify-center rounded-full transition-colors duration-100"
          style={{ background: "rgba(0,0,0,0.5)", color: "#fff" }}
        >
          <X size={8} strokeWidth={2.5} />
        </button>
      </div>
    );
  }

  return (
    <div
      className="flex min-w-0 max-w-full items-center gap-1.5 rounded-lg px-2.5 py-1.5 text-[12px]"
      style={{
        background: "var(--bg-secondary)",
        border: `0.5px solid var(--separator)`,
        animation: "pop 0.2s ease-out",
      }}
    >
      <span className="shrink-0" style={{ color: "var(--fill-tertiary)" }}>{icon}</span>
      <span className="min-w-0 max-w-[120px] truncate" style={{ color: "var(--fill-primary)" }} title={file.name}>{file.name}</span>
      <span className="shrink-0" style={{ color: "var(--fill-quaternary)" }}>{formatSize(file.size)}</span>
      <button
        onClick={onRemove}
        className="ml-0.5 flex h-4 w-4 cursor-pointer items-center justify-center rounded-full transition-colors duration-100 hover:bg-[var(--bg-hover)]"
        style={{ color: "var(--fill-tertiary)" }}
      >
        <X size={8} strokeWidth={2.5} />
      </button>
    </div>
  );
}

/* ─── (MentionInput is now in ./MentionInput.tsx) ─── */

/* ─── Keyboard Shortcuts Hint ─── */
function ShortcutsHint() {
  const isMac = navigator.platform.includes("Mac");
  const mod = isMac ? "⌘" : "Ctrl";

  return (
    <div
      className="flex items-center gap-3 text-[11px]"
      style={{ color: "var(--fill-quaternary)" }}
    >
      <span><kbd className="font-mono text-[10px]">Enter</kbd> 发送</span>
      <span><kbd className="font-mono text-[10px]">Shift+Enter</kbd> 换行</span>
      <span><kbd className="font-mono text-[10px]">{mod}+K</kbd> 新话题</span>
    </div>
  );
}

/* ─── Chat Tabs Bar ─── */

import type { Chat } from "../../lib/agent-store";

interface ChatTabsBarProps {
  agentId: string;
  chats: Chat[];
  activeChatId: string;
  streamingChatIds: Set<string>;
  onSelect: (id: string) => void;
  onClose: (id: string) => void;
  onNew: () => void;
  onRename: (id: string, title: string) => void;
  onReorder: (fromIdx: number, toIdx: number) => void;
}

function ChatTabsBar({ chats, activeChatId, streamingChatIds, onSelect, onClose, onNew, onRename, onReorder }: ChatTabsBarProps) {
  const [editingId, setEditingId] = useState<string | null>(null);
  const [editValue, setEditValue] = useState("");
  const editRef = useRef<HTMLInputElement>(null);
  const [dragId, setDragId] = useState<string | null>(null);
  const [dropTarget, setDropTarget] = useState<string | null>(null);
  const [hoveredTab, setHoveredTab] = useState<string | null>(null);

  const openChats = chats.filter((c) => c.open);

  const handleDblClick = (chat: Chat) => {
    setEditingId(chat.id);
    setEditValue(chat.title);
    setTimeout(() => editRef.current?.select(), 0);
  };

  const commitRename = () => {
    if (editingId && editValue.trim()) {
      onRename(editingId, editValue.trim());
    }
    setEditingId(null);
  };

  const handleDragStart = (e: React.DragEvent, chatId: string) => {
    setDragId(chatId);
    e.dataTransfer.effectAllowed = "move";
    e.dataTransfer.setData("text/plain", chatId);
    if (e.currentTarget instanceof HTMLElement) {
      e.currentTarget.style.opacity = "0.5";
    }
  };

  const handleDragEnd = (e: React.DragEvent) => {
    setDragId(null);
    setDropTarget(null);
    if (e.currentTarget instanceof HTMLElement) {
      e.currentTarget.style.opacity = "1";
    }
  };

  const handleDragOver = (e: React.DragEvent, chatId: string) => {
    e.preventDefault();
    e.dataTransfer.dropEffect = "move";
    if (chatId !== dragId) setDropTarget(chatId);
  };

  const handleDrop = (e: React.DragEvent, targetChatId: string) => {
    e.preventDefault();
    if (!dragId || dragId === targetChatId) return;
    const fromIdx = openChats.findIndex((c) => c.id === dragId);
    const toIdx = openChats.findIndex((c) => c.id === targetChatId);
    if (fromIdx >= 0 && toIdx >= 0) onReorder(fromIdx, toIdx);
    setDragId(null);
    setDropTarget(null);
  };

  return (
    <div
      className="hide-scrollbar flex shrink-0 items-center gap-0 overflow-x-auto px-1"
      style={{
        background: "var(--bg-secondary)",
        borderBottom: `0.5px solid var(--separator)`,
        height: 36,
      }}
    >
      {openChats.map((chat) => {
        const isActive = chat.id === activeChatId;
        const isDropHere = dropTarget === chat.id;
        return (
          <div
            key={chat.id}
            draggable={editingId !== chat.id}
            onDragStart={(e) => handleDragStart(e, chat.id)}
            onDragEnd={handleDragEnd}
            onDragOver={(e) => handleDragOver(e, chat.id)}
            onDragLeave={() => setDropTarget(null)}
            onDrop={(e) => handleDrop(e, chat.id)}
            className="relative flex shrink-0 items-center gap-1 rounded-t-lg px-3 text-[12px] transition-all duration-150"
            style={{
              height: "calc(100% - 2px)",
              marginTop: 2,
              color: isActive ? "var(--fill-primary)" : "var(--fill-tertiary)",
              borderBottom: isActive ? `2px solid var(--tint)` : "2px solid transparent",
              background: isActive ? "var(--bg-primary)" : hoveredTab === chat.id ? "var(--bg-hover)" : "transparent",
              fontWeight: isActive ? 600 : 400,
              cursor: editingId === chat.id ? "text" : "pointer",
              borderLeft: isDropHere ? `2px solid var(--tint)` : "2px solid transparent",
            }}
            title={chat.workDir ? `${chat.title}\n${chat.workDir.replace(/^\/home\/[^/]+\//, "~/")}` : chat.title}
            onClick={() => { if (editingId !== chat.id) onSelect(chat.id); }}
            onDoubleClick={() => handleDblClick(chat)}
            onMouseEnter={() => setHoveredTab(chat.id)}
            onMouseLeave={() => setHoveredTab(null)}
          >
            {editingId === chat.id ? (
              <input
                ref={editRef}
                value={editValue}
                onChange={(e) => setEditValue(e.target.value)}
                onBlur={commitRename}
                onKeyDown={(e) => {
                  if (e.key === "Enter") commitRename();
                  if (e.key === "Escape") setEditingId(null);
                }}
                className="w-[100px] rounded-sm bg-transparent px-0.5 text-[12px] font-medium outline-none ring-1 ring-[var(--tint)]"
                style={{ color: "var(--fill-primary)" }}
                onClick={(e) => e.stopPropagation()}
              />
            ) : (
              <>
                {streamingChatIds.has(chat.id) && !isActive && (
                  <span
                    className="mr-1 inline-block h-[5px] w-[5px] shrink-0 rounded-full"
                    style={{ background: "var(--tint)", animation: "cursor-blink 1s step-end infinite" }}
                  />
                )}
                <span className="max-w-[120px] truncate select-none">{chat.title}</span>
              </>
            )}
            <button
              onClick={(e) => { e.stopPropagation(); onClose(chat.id); }}
              className="ml-0.5 flex h-4 w-4 items-center justify-center rounded-sm transition-opacity duration-100"
              style={{
                color: "var(--fill-quaternary)",
                opacity: hoveredTab === chat.id ? 1 : 0,
              }}
            >
              <X size={8} strokeWidth={2.5} />
            </button>
          </div>
        );
      })}
      <button
        onClick={onNew}
        className="ml-1 flex h-6 w-6 shrink-0 items-center justify-center rounded-md transition-colors duration-100 hover:bg-[var(--bg-hover)]"
        style={{ color: "var(--fill-quaternary)" }}
        title="新建会话"
      >
        <MessageSquare size={12} strokeWidth={1.5} />
      </button>
    </div>
  );
}

/* ─── Empty State ─── */
function EmptyState({ onPick }: { onPick: (t: string) => void }) {
  const suggestions = [
    { text: "帮我分析一段代码", icon: <FileText size={14} strokeWidth={1.5} /> },
    { text: "写一个 API 设计方案", icon: <Sparkles size={14} strokeWidth={1.5} /> },
    { text: "排查一个 Bug", icon: <File size={14} strokeWidth={1.5} /> },
    { text: "优化系统性能", icon: <Folder size={14} strokeWidth={1.5} /> },
  ];

  return (
    <div className="flex h-full flex-col items-center justify-center px-8" style={{ animation: "scale-in 0.35s ease-out" }}>
      <div className="mb-8">
        <ClawIcon size={56} />
      </div>
      <h2 className="mb-2 text-[17px] font-semibold tracking-[-0.02em]" style={{ color: "var(--fill-primary)" }}>
        开始新的对话
      </h2>
      <p className="mb-10 text-[13px]" style={{ color: "var(--fill-tertiary)" }}>
        描述你的任务，或选择一个话题
      </p>
      <div className="grid grid-cols-2 gap-2.5" style={{ maxWidth: 380 }}>
        {suggestions.map((s, i) => (
          <button
            key={s.text}
            onClick={() => onPick(s.text)}
            className="flex cursor-pointer items-center gap-2.5 rounded-[var(--radius-sm)] px-4 py-3 text-left text-[13px] transition-all duration-150 hover:bg-[var(--bg-tertiary)]"
            style={{
              background: "var(--bg-secondary)",
              border: "0.5px solid var(--separator)",
              color: "var(--fill-secondary)",
              animation: `slide-up 0.3s ease-out ${0.06 + i * 0.05}s backwards`,
            }}
          >
            <span className="shrink-0" style={{ color: "var(--fill-tertiary)" }}>{s.icon}</span>
            <span className="min-w-0 truncate">{s.text}</span>
          </button>
        ))}
      </div>
    </div>
  );
}

const REF_PATTERN = /\n\n\[(引用|附件): ([^\]]+)\]$/;

function parseUserContent(content: string): { text: string; tags: Array<{ type: string; items: string[] }> } {
  const tags: Array<{ type: string; items: string[] }> = [];
  let text = content;
  let match: RegExpExecArray | null;
  while ((match = REF_PATTERN.exec(text)) !== null) {
    tags.unshift({ type: match[1], items: match[2].split(", ") });
    text = text.slice(0, match.index);
  }
  return { text, tags };
}

/* ─── User Message ─── */
function UserBubble({ msg }: { msg: ChatMessage }) {
  const { text, tags } = useMemo(() => parseUserContent(msg.content), [msg.content]);
  return (
    <div className="pb-5 flex justify-end" style={{ animation: "slide-right 0.2s ease-out" }}>
      <div className="flex flex-col items-end" style={{ maxWidth: "65%" }}>
        <div
          className="rounded-2xl px-4 py-3 text-[15px] leading-[1.6] break-words"
          style={{
            background: "var(--bubble-user)",
            color: "var(--bubble-user-text)",
            borderTopRightRadius: 6,
            overflowWrap: "anywhere",
          }}
        >
          {text}
          {msg.images && msg.images.length > 0 && (
            <div className="mt-2 flex flex-wrap gap-1.5">
              {msg.images.map((img, i) => (
                <img
                  key={i}
                  src={img.url}
                  alt={img.alt || "attached image"}
                  className="rounded-md object-cover"
                  style={{
                    maxHeight: 200,
                    maxWidth: "100%",
                    border: "0.5px solid rgba(255,255,255,0.2)",
                  }}
                  loading="lazy"
                />
              ))}
            </div>
          )}
          {tags.length > 0 && (
            <div className="mt-2 flex flex-wrap gap-1.5">
              {tags.map((tag, ti) =>
                tag.items.map((item, ii) => (
                  <span
                    key={`${ti}-${ii}`}
                    className="inline-flex items-center gap-1 rounded-md px-2 py-0.5 text-[11px] font-medium"
                    style={{
                      background: "rgba(255,255,255,0.15)",
                      color: "var(--bubble-user-text)",
                      border: "0.5px solid rgba(255,255,255,0.2)",
                    }}
                  >
                    <span className="text-[10px]">{tag.type === "引用" ? "📎" : "📄"}</span>
                    <span className="max-w-[120px] truncate">{item}</span>
                  </span>
                ))
              )}
            </div>
          )}
        </div>
        <span className="mt-1 pr-1 text-[10px]" style={{ color: "var(--fill-quaternary)" }}>
          {ts(msg.timestamp)}
        </span>
      </div>
    </div>
  );
}

/* ─── AI Message ─── */
function AiMessage({ msg, usage }: { msg: ChatMessage; usage?: ChatUsage }) {
  const toolCalls = msg.toolCalls;
  return (
    <div className="pb-7" style={{ animation: "slide-left 0.2s ease-out", maxWidth: "75%" }}>
      {toolCalls && toolCalls.length > 0 && (
        <div className="mb-2">
          {toolCalls.map((tc) => (
            <ToolCallCard
              key={tc.id}
              tool={{ ...tc, status: tc.status as "running" | "success" | "error" }}
            />
          ))}
        </div>
      )}
      <MarkdownContent content={msg.content} />
      <div className="mt-1 flex items-center gap-2.5 text-[10px]" style={{ color: "var(--fill-quaternary)" }}>
        <span>{ts(msg.timestamp)}</span>
        {usage && (
          <>
            <span className="flex items-center gap-0.5" title="耗时">
              <Clock size={9} strokeWidth={1.5} />
              {formatElapsed(usage.elapsedMs)}
            </span>
            <span className="flex items-center gap-0.5" title="上行 Token">
              <ArrowUpRight size={9} strokeWidth={1.5} />
              {formatTokens(usage.promptTokens)}
            </span>
            <span className="flex items-center gap-0.5" title="下行 Token">
              <ArrowDownRight size={9} strokeWidth={1.5} />
              {formatTokens(usage.completionTokens)}
            </span>
          </>
        )}
      </div>
    </div>
  );
}

function SystemMsg({ msg }: { msg: ChatMessage }) {
  return (
    <div className="pb-4 break-words rounded-xl px-3 py-2 text-[13px]" style={{ background: "var(--tint-subtle)", color: "var(--fill-secondary)", overflowWrap: "anywhere" }}>
      {msg.content}
    </div>
  );
}

/* StreamText removed — streaming text now rendered inline via streamSegments */

const OPTION_LETTERS = "ABCDEFGHIJKLMNOPQRSTUVWXYZ";

function QuestionPanel({
  question,
  onAnswer,
  onTimeout,
}: {
  question: {
    requestId: string;
    question: string;
    options: Array<{ id: string; label: string }>;
    timeoutSecs: number;
    expiresAt: number;
    allowMultiple?: boolean;
  };
  onAnswer: (answer: string) => void;
  onTimeout: () => void;
}) {
  const [remaining, setRemaining] = useState(() => Math.max(0, Math.ceil((question.expiresAt - Date.now()) / 1000)));
  const [freeText, setFreeText] = useState("");
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [submitted, setSubmitted] = useState(false);
  const panelRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const timer = setInterval(() => {
      const left = Math.max(0, Math.ceil((question.expiresAt - Date.now()) / 1000));
      setRemaining(left);
      if (left <= 0) {
        clearInterval(timer);
        onTimeout();
      }
    }, 200);
    return () => clearInterval(timer);
  }, [question.expiresAt, onTimeout]);

  const progress = Math.max(0, remaining / question.timeoutSecs);
  const multi = question.allowMultiple ?? false;

  const handleOptionClick = useCallback((optId: string) => {
    if (submitted) return;
    if (multi) {
      setSelected((prev) => {
        const next = new Set(prev);
        if (next.has(optId)) next.delete(optId);
        else next.add(optId);
        return next;
      });
    } else {
      setSubmitted(true);
      onAnswer(optId);
    }
  }, [submitted, multi, onAnswer]);

  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (submitted) return;
      const key = e.key.toUpperCase();
      const idx = OPTION_LETTERS.indexOf(key);
      if (idx >= 0 && idx < question.options.length) {
        e.preventDefault();
        handleOptionClick(question.options[idx].id);
      }
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [submitted, question.options, handleOptionClick]);

  const handleSubmitMulti = () => {
    if (selected.size > 0 && !submitted) {
      setSubmitted(true);
      onAnswer(Array.from(selected).join(","));
    }
  };

  const handleFreeTextSubmit = () => {
    if (freeText.trim() && !submitted) {
      setSubmitted(true);
      onAnswer(freeText.trim());
    }
  };

  const reducedMotion = typeof window !== "undefined" && window.matchMedia?.("(prefers-reduced-motion: reduce)").matches;

  return (
    <div
      ref={panelRef}
      role="dialog"
      aria-label={question.question}
      className="mb-2 overflow-hidden rounded-xl"
      style={{
        background: "var(--bg-elevated)",
        border: "1px solid var(--separator-opaque)",
        boxShadow: "var(--shadow-sm)",
        animation: reducedMotion ? "none" : "slide-up 0.2s ease-out",
      }}
    >
      <div className="relative h-[2px] w-full" style={{ background: "var(--bg-tertiary)" }}>
        <div
          className="absolute inset-y-0 left-0 transition-all duration-200"
          style={{ width: `${progress * 100}%`, background: remaining <= 10 ? "var(--fill-warning, #ED8936)" : "var(--fill-accent, #4299E1)" }}
        />
      </div>
      <div className="px-4 py-3">
        <div className="mb-3 flex items-center justify-between gap-2">
          <p className="text-[13px] font-medium" style={{ color: "var(--fill-primary)" }}>{question.question}</p>
          <span className="shrink-0 text-[11px] tabular-nums" style={{ color: remaining <= 10 ? "var(--fill-warning, #ED8936)" : "var(--fill-tertiary)" }}>
            {remaining}s
          </span>
        </div>
        {multi && (
          <p className="mb-2 text-[11px]" style={{ color: "var(--fill-tertiary)" }}>可多选，选完后点击"确认"&nbsp;·&nbsp;按键盘 A-{OPTION_LETTERS[Math.min(question.options.length, OPTION_LETTERS.length) - 1]} 快速选择</p>
        )}
        {!multi && question.options.length > 0 && (
          <p className="mb-2 text-[11px]" style={{ color: "var(--fill-tertiary)" }}>按键盘 A-{OPTION_LETTERS[Math.min(question.options.length, OPTION_LETTERS.length) - 1]} 快速选择</p>
        )}
        <div className="flex flex-col gap-1.5" role="group" aria-label="选项列表">
          {question.options.map((opt, idx) => {
            const letter = OPTION_LETTERS[idx] ?? String(idx + 1);
            const isSelected = selected.has(opt.id);
            return (
              <button
                key={opt.id}
                onClick={() => handleOptionClick(opt.id)}
                disabled={submitted}
                aria-label={`选项 ${letter}: ${opt.label}`}
                aria-pressed={multi ? isSelected : undefined}
                className="flex w-full cursor-pointer items-center gap-2.5 rounded-lg px-3 py-2 text-left text-[12px] transition-colors duration-150 focus-visible:ring-2 focus-visible:ring-[var(--fill-accent,#4299E1)] focus-visible:ring-offset-1 disabled:cursor-not-allowed disabled:opacity-50"
                style={{
                  background: isSelected ? "var(--tint-bg, rgba(66,153,225,0.15))" : "var(--bg-primary)",
                  color: "var(--fill-primary)",
                  border: `1px solid ${isSelected ? "var(--fill-accent, #4299E1)" : "var(--separator)"}`,
                }}
              >
                <span
                  className="flex h-5 w-5 shrink-0 items-center justify-center rounded text-[11px] font-semibold transition-colors duration-150"
                  style={{
                    background: isSelected ? "var(--fill-accent, #4299E1)" : "var(--bg-tertiary)",
                    color: isSelected ? "#fff" : "var(--fill-secondary)",
                  }}
                >
                  {letter}
                </span>
                <span className="font-medium">{opt.label}</span>
              </button>
            );
          })}
        </div>
        {multi && selected.size > 0 && (
          <div className="mt-2 flex justify-end">
            <button
              onClick={handleSubmitMulti}
              disabled={submitted}
              className="cursor-pointer rounded-lg px-4 py-1.5 text-[12px] font-medium transition-colors duration-150 hover:opacity-80 focus-visible:ring-2 focus-visible:ring-[var(--fill-accent,#4299E1)] focus-visible:ring-offset-1 disabled:cursor-not-allowed disabled:opacity-50"
              style={{ background: "var(--fill-primary)", color: "var(--fill-inverse)" }}
            >
              确认（{selected.size}项）
            </button>
          </div>
        )}
        <div className="mt-2 flex gap-2">
          <input
            type="text"
            value={freeText}
            onChange={(e) => setFreeText(e.target.value)}
            onKeyDown={(e) => { if (e.key === "Enter") handleFreeTextSubmit(); }}
            disabled={submitted}
            placeholder="或输入自定义回答..."
            aria-label="自定义回答"
            className="min-w-0 flex-1 rounded-lg px-2.5 py-1.5 text-[12px] outline-none transition-colors duration-150 focus-visible:ring-2 focus-visible:ring-[var(--fill-accent,#4299E1)] focus-visible:ring-offset-1 disabled:cursor-not-allowed disabled:opacity-50"
            style={{ background: "var(--bg-primary)", color: "var(--fill-primary)", border: "1px solid var(--separator)" }}
          />
          {freeText.trim() && (
            <button
              onClick={handleFreeTextSubmit}
              disabled={submitted}
              className="cursor-pointer rounded-lg px-3 py-1.5 text-[12px] font-medium transition-colors duration-150 hover:opacity-80 focus-visible:ring-2 focus-visible:ring-[var(--fill-accent,#4299E1)] focus-visible:ring-offset-1 disabled:cursor-not-allowed disabled:opacity-50"
              style={{ background: "var(--fill-primary)", color: "var(--fill-inverse)" }}
            >
              发送
            </button>
          )}
        </div>
      </div>
    </div>
  );
}

function formatElapsed(ms: number): string {
  if (ms < 1000) return `${ms}ms`;
  const secs = ms / 1000;
  if (secs < 60) return `${secs.toFixed(1)}s`;
  const mins = Math.floor(secs / 60);
  const remSecs = Math.round(secs % 60);
  return `${mins}m ${remSecs}s`;
}

function formatTokens(n: number): string {
  if (n < 1000) return String(n);
  if (n < 1_000_000) return `${(n / 1000).toFixed(1)}k`;
  return `${(n / 1_000_000).toFixed(2)}M`;
}

function ContextRing({ used, limit }: { used: number; limit: number }) {
  const [hover, setHover] = useState(false);
  const ratio = limit > 0 ? used / limit : 0;
  const clampedRatio = Math.min(ratio, 1);
  const pct = clampedRatio * 100;
  const color = ratio < 0.5
    ? "var(--green, #68D391)"
    : ratio < 0.8
      ? "var(--yellow, #ED8936)"
      : "var(--red, #FC8181)";

  const size = 24;
  const strokeWidth = 2.5;
  const r = (size - strokeWidth) / 2;
  const circumference = 2 * Math.PI * r;
  const offset = circumference * (1 - clampedRatio);

  const remaining = Math.max(0, limit - used);
  const warning = ratio >= 0.8;
  const critical = ratio >= 0.95;

  return (
    <div
      className="relative flex items-center justify-center"
      style={{
        width: size,
        height: size,
        cursor: "default",
        animation: critical ? "pulse 2s ease-in-out infinite" : undefined,
      }}
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => setHover(false)}
    >
      <svg width={size} height={size} style={{ transform: "rotate(-90deg)" }}>
        <circle
          cx={size / 2} cy={size / 2} r={r}
          fill="none"
          stroke="var(--separator-opaque, #E2E8F0)"
          strokeWidth={strokeWidth}
          opacity={0.6}
        />
        <circle
          cx={size / 2} cy={size / 2} r={r}
          fill="none"
          stroke={color}
          strokeWidth={strokeWidth}
          strokeDasharray={circumference}
          strokeDashoffset={offset}
          strokeLinecap="round"
          style={{ transition: "stroke-dashoffset 0.4s ease, stroke 0.3s ease" }}
        />
      </svg>
      <span
        className="absolute text-[7px] font-bold tabular-nums leading-none"
        style={{ color }}
      >
        {pct < 1 ? "<1" : Math.round(pct)}
      </span>
      {hover && (
        <div
          className="absolute bottom-full mb-2 rounded-xl px-3 py-2.5"
          style={{
            background: "var(--bg-elevated)",
            border: "1px solid var(--separator)",
            boxShadow: "var(--shadow-lg)",
            color: "var(--fill-primary)",
            zIndex: 50,
            right: -8,
            minWidth: 180,
            animation: "fade-in 0.1s ease-out",
          }}
        >
          <div className="mb-1.5 text-[11px] font-semibold" style={{ color: "var(--fill-secondary)" }}>
            上下文窗口
          </div>
          <div className="mb-2 flex items-baseline gap-1">
            <span className="text-[16px] font-bold tabular-nums" style={{ color }}>{formatTokens(used)}</span>
            <span className="text-[11px]" style={{ color: "var(--fill-quaternary)" }}>/ {formatTokens(limit)} tokens</span>
          </div>
          <div
            className="mb-2 h-[4px] w-full overflow-hidden rounded-full"
            style={{ background: "var(--separator-opaque, #E2E8F0)" }}
          >
            <div
              className="h-full rounded-full"
              style={{
                width: `${pct}%`,
                background: color,
                transition: "width 0.3s ease",
              }}
            />
          </div>
          <div className="flex justify-between text-[10px]" style={{ color: "var(--fill-tertiary)" }}>
            <span>已用 {pct.toFixed(1)}%</span>
            <span>剩余 {formatTokens(remaining)}</span>
          </div>
          {warning && (
            <div
              className="mt-2 rounded-md px-2 py-1 text-[10px]"
              style={{
                background: critical ? "rgba(252,129,129,0.12)" : "rgba(237,137,54,0.12)",
                color: critical ? "var(--red, #FC8181)" : "var(--yellow, #ED8936)",
              }}
            >
              {critical ? "上下文即将溢出，建议开始新对话" : "上下文使用较高，较长对话可能被压缩"}
            </div>
          )}
        </div>
      )}
    </div>
  );
}

function Typing() {
  return (
    <div className="pb-6 flex items-center gap-1" style={{ animation: "fade-in 0.15s" }}>
      {[0, 1, 2].map((i) => (
        <div
          key={i}
          className="h-[5px] w-[5px] rounded-full"
          style={{ background: "var(--fill-tertiary)", animation: `typing-bounce 1.4s ease-in-out ${i * 0.18}s infinite` }}
        />
      ))}
    </div>
  );
}

/* ━━━ MessageStream ━━━ */
interface MessageStreamProps {
  onToggleDetail?: () => void;
  detailOpen?: boolean;
}

export function MessageStream({ onToggleDetail, detailOpen }: MessageStreamProps) {
  const activeAgentId = useAgentStore((s) => s.activeAgentId);
  const agents = useAgentStore((s) => s.agents);
  const agentChats = useAgentStore((s) => s.agentChats);
  const addMessage = useAgentStore((s) => s.addMessage);
  const newChat = useAgentStore((s) => s.newChat);
  const setWorkDir = useAgentStore((s) => s.setWorkDir);
  const setActiveChat = useAgentStore((s) => s.setActiveChat);
  const closeChat = useAgentStore((s) => s.closeChat);
  const renameChat = useAgentStore((s) => s.renameChat);
  const reorderChats = useAgentStore((s) => s.reorderChats);

  const loadChatStream = useAgentStore((s) => s.loadChatStream);
  const updateChatBackendId = useAgentStore((s) => s.updateChatBackendId);
  const updateChatUsage = useAgentStore((s) => s.updateChatUsage);

  const agent = agents.find((a) => a.id === activeAgentId) ?? agents[0];
  const ac = agentChats[activeAgentId];
  const activeChat = ac?.chatList.find((c) => c.id === ac.activeChatId);
  const stream = activeChat?.stream ?? [];
  const workDir = activeChat?.workDir ?? null;

  const loadingChats = useRef(new Set<string>());
  const loadedChats = useRef(new Set<string>());
  useEffect(() => {
    if (!activeChat) return;
    if (activeChat.messageCount === 0 && activeChat.stream.length === 0) return;
    if (loadingChats.current.has(activeChat.id)) return;
    if (loadedChats.current.has(activeChat.id) && activeChat.stream.length > 0) return;

    loadingChats.current.add(activeChat.id);
    transport.getSessionMessages(activeChat.id).then((messages) => {
      if (messages && messages.length > 0 && messages.length > activeChat.stream.length) {
        loadChatStream(activeAgentId, activeChat.id, messages);
      }
      loadedChats.current.add(activeChat.id);
    }).catch(() => {}).finally(() => {
      loadingChats.current.delete(activeChat.id);
    });
  }, [activeChat?.id, activeChat?.messageCount, activeChat?.stream.length, activeAgentId, loadChatStream]);

  interface StreamSegment {
    id: string;
    type: "text" | "tool";
    content?: string;
    toolCall?: ToolCall;
  }

  const [streaming, setStreaming] = useState(false);
  const [streamSegments, setStreamSegments] = useState<StreamSegment[]>([]);
  const segmentsRef = useRef<StreamSegment[]>([]);
  const [pendingQuestion, setPendingQuestion] = useState<{
    requestId: string;
    question: string;
    options: Array<{ id: string; label: string }>;
    timeoutSecs: number;
    expiresAt: number;
    allowMultiple?: boolean;
  } | null>(null);
  const streamAccRef = useRef("");
  const rafIdRef = useRef(0);
  const currentStreamChatRef = useRef<string | null>(null);
  const cleanupRef = useRef<(() => void) | null>(null);
  const bottomRef = useRef<HTMLDivElement>(null);
  const virtuosoRef = useRef<VirtuosoHandle>(null);
  const scrollPositions = useRef<Record<string, number>>({});
  const mentionInputRef = useRef<MentionInputHandle>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);

  // File attachments
  const [attachedFiles, setAttachedFiles] = useState<AttachedFile[]>([]);
  const [isDragging, setIsDragging] = useState(false);
  const dragCounter = useRef(0);

  // Search state
  const [searchOpen, setSearchOpen] = useState(false);
  const [searchQuery, setSearchQuery] = useState("");
  const [searchIdx, setSearchIdx] = useState(0);
  const searchInputRef = useRef<HTMLInputElement>(null);
  const chatScrollKey = useCallback((chatId: string | undefined) => {
    if (!chatId) return undefined;
    const chat = ac?.chatList.find((c) => c.id === chatId);
    const stableKey = chat?.localKey ?? chatId;
    return `${activeAgentId}:${stableKey}`;
  }, [ac?.chatList, activeAgentId]);

  // Detach / reattach streams on chat switch
  const prevChatIdRef = useRef<string | undefined>(undefined);
  useEffect(() => {
    const prevId = prevChatIdRef.current;
    const newId = activeChat?.id;
    prevChatIdRef.current = newId;

    if (prevId && prevId !== newId && streaming) {
      // Detach: save current streaming state to the registry
      if (detachedStreams.size >= MAX_DETACHED_STREAMS) {
        const oldestKey = detachedStreams.keys().next().value;
        if (oldestKey) detachedStreams.delete(oldestKey);
      }
      detachedStreams.set(prevId, {
        agentId: activeAgentId,
        chatId: prevId,
        acc: streamAccRef.current,
        toolCalls: segmentsRef.current.filter((s) => s.type === "tool" && s.toolCall).map((s) => s.toolCall!),
        scrollPosition: (() => {
          const key = chatScrollKey(prevId);
          return key ? scrollPositions.current[key] : undefined;
        })(),
        done: false,
        error: false,
        cleanup: () => {},
      });
      currentStreamChatRef.current = null;
      setStreaming(false);
      segmentsRef.current = [];
      setStreamSegments([]);
      pendingBottomScrollBehaviorRef.current = null;
    }

    if (newId && detachedStreams.has(newId)) {
      const ds = detachedStreams.get(newId)!;
      if (ds.done) {
        detachedStreams.delete(newId);
      } else {
        streamAccRef.current = ds.acc;
        currentStreamChatRef.current = newId;
        const restored: StreamSegment[] = [];
        if (ds.acc) restored.push({ id: "text-0", type: "text", content: ds.acc });
        for (const tc of ds.toolCalls) {
          restored.push({ id: `tool-${tc.id}`, type: "tool", toolCall: tc });
        }
        segmentsRef.current = restored;
        setStreamSegments([...restored]);
        setStreaming(true);
        if (ds.scrollPosition != null) {
          const key = chatScrollKey(newId);
          if (key) scrollPositions.current[key] = ds.scrollPosition;
        }
        detachedStreams.delete(newId);
      }
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [activeChat?.id, chatScrollKey]);

  useEffect(() => {
    return () => {
      // Prevent detached stream registry from retaining stale closures after unmount.
      for (const [chatId, ds] of detachedStreams) {
        if (ds.agentId === activeAgentId) {
          detachedStreams.delete(chatId);
        }
      }
    };
  }, [activeAgentId]);

  const searchResults = useMemo(() => {
    if (!searchQuery.trim()) return [];
    const q = searchQuery.toLowerCase();
    return stream
      .map((item, idx) => ({ item, idx }))
      .filter(({ item }) => item.data.content.toLowerCase().includes(q));
  }, [stream, searchQuery]);

  const openSearch = useCallback(() => {
    setSearchOpen(true);
    setSearchQuery("");
    setSearchIdx(0);
    setTimeout(() => searchInputRef.current?.focus(), 0);
  }, []);

  const closeSearch = useCallback(() => {
    setSearchOpen(false);
    setSearchQuery("");
  }, []);

  const paginationOffsetRef = useRef(0);

  useEffect(() => {
    const handleGlobalKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key === "f") {
        e.preventDefault();
        if (searchOpen) closeSearch();
        else openSearch();
      }
    };
    window.addEventListener("keydown", handleGlobalKey);
    return () => window.removeEventListener("keydown", handleGlobalKey);
  }, [searchOpen, openSearch, closeSearch]);

  // Real file listing from Tauri FS
  const [fsEntries, setFsEntries] = useState<{ files: string[]; dirs: string[] }>({ files: [], dirs: [] });
  useEffect(() => {
    if (!workDir) { setFsEntries({ files: [], dirs: [] }); return; }
    api.listFiles(workDir).then(setFsEntries).catch(() => setFsEntries({ files: [], dirs: [] }));
  }, [workDir]);

  // Skills from backend
  const [backendSkills, setBackendSkills] = useState<api.SkillInfo[]>([]);
  useEffect(() => {
    api.listSkills().then(setBackendSkills).catch(() => {});
  }, []);

  const mentionOptions: MentionOption[] = useMemo(() => {
    const opts: MentionOption[] = [];

    if (workDir) {
      for (const f of fsEntries.files) {
        opts.push({ id: `f-${f}`, label: f, type: "file", desc: f });
      }
      for (const d of fsEntries.dirs) {
        opts.push({ id: `d-${d}`, label: `${d}/`, type: "dir", desc: `${d}/` });
      }
    }

    if (backendSkills.length > 0) {
      for (const s of backendSkills) {
        opts.push({ id: `s-${s.id}`, label: s.name, type: "skill", desc: s.description });
      }
    } else {
      opts.push(
        { id: "s-web-search", label: "Web Search", type: "skill", desc: "搜索互联网获取实时信息" },
        { id: "s-code-exec", label: "Code Execution", type: "skill", desc: "在沙箱中执行代码片段" },
      );
    }

    return opts;
  }, [workDir, fsEntries, backendSkills]);

  const streamingChatIds = useMemo(() => {
    const ids = new Set<string>();
    if (streaming && currentStreamChatRef.current) ids.add(currentStreamChatRef.current);
    for (const [chatId, ds] of detachedStreams) {
      if (!ds.done) ids.add(chatId);
    }
    return ids;
  }, [streaming, streamSegments]);

  const atBottomRef = useRef(true);
  const suppressScrollTrackingUntilRef = useRef(0);
  const pendingBottomScrollBehaviorRef = useRef<"auto" | "smooth" | null>(null);
  const pendingRestoreScrollTopRef = useRef<number | null>(null);
  const runProgrammaticScroll = useCallback((action: () => void, suppressMs = 280) => {
    suppressScrollTrackingUntilRef.current = Date.now() + suppressMs;
    action();
  }, []);
  const requestBottomScroll = useCallback((behavior: "auto" | "smooth") => {
    pendingBottomScrollBehaviorRef.current = behavior;
  }, []);
  const handleAtBottomChange = useCallback((atBottom: boolean) => {
    atBottomRef.current = atBottom;
  }, []);

  // File drag handlers
  const handleDragEnter = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    e.stopPropagation();
    dragCounter.current++;
    if (e.dataTransfer.types.includes("Files")) {
      setIsDragging(true);
    }
  }, []);

  const handleDragLeave = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    e.stopPropagation();
    dragCounter.current--;
    if (dragCounter.current === 0) {
      setIsDragging(false);
    }
  }, []);

  const handleDragOver = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    e.stopPropagation();
  }, []);

  const processFiles = useCallback((files: FileList | File[]) => {
    const newFiles: AttachedFile[] = Array.from(files).map((f) => ({
      name: f.name,
      size: f.size,
      type: f.type,
      file: f,
      previewUrl: f.type.startsWith("image/") ? URL.createObjectURL(f) : undefined,
    }));
    setAttachedFiles((prev) => [...prev, ...newFiles]);
  }, []);

  const handleDrop = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    e.stopPropagation();
    dragCounter.current = 0;
    setIsDragging(false);
    if (e.dataTransfer.files.length > 0) {
      processFiles(e.dataTransfer.files);
    }
  }, [processFiles]);

  const removeFile = useCallback((index: number) => {
    setAttachedFiles((prev) => {
      const removed = prev[index];
      if (removed?.previewUrl) URL.revokeObjectURL(removed.previewUrl);
      return prev.filter((_, i) => i !== index);
    });
  }, []);

  const sendWithContent = async (txt: string, mentions: InlineMention[]) => {
    const mentionDesc = mentions.length > 0
      ? `\n\n[引用: ${mentions.map((m) => `@${m.label} (${m.type})`).join(", ")}]`
      : "";
    const nonImageFiles = attachedFiles.filter((f) => !f.type.startsWith("image/"));
    const imageFiles = attachedFiles.filter((f) => f.type.startsWith("image/"));
    const fileDesc = nonImageFiles.length > 0
      ? `\n\n[附件: ${nonImageFiles.map((f) => f.name).join(", ")}]`
      : "";

    const imageDataUrls: Array<{ url: string; alt?: string }> = [];
    if (imageFiles.length > 0) {
      await Promise.all(
        imageFiles.map(
          (af) =>
            new Promise<void>((resolve) => {
              const reader = new FileReader();
              reader.onload = () => {
                if (typeof reader.result === "string") {
                  imageDataUrls.push({ url: reader.result, alt: af.name });
                }
                resolve();
              };
              reader.onerror = () => resolve();
              reader.readAsDataURL(af.file);
            }),
        ),
      );
    }

    const currentState = useAgentStore.getState();
    const capturedAgentId = currentState.activeAgentId;
    const currentAc = currentState.agentChats[capturedAgentId];
    const currentActiveChat = currentAc?.chatList.find((c) => c.id === currentAc.activeChatId);
    const capturedChatId = currentActiveChat?.id ?? "temp";

    addMessage(capturedAgentId, {
      role: "user",
      content: txt + mentionDesc + fileDesc,
      timestamp: new Date(),
      images: imageDataUrls.length > 0 ? imageDataUrls : undefined,
    }, capturedChatId);
    atBottomRef.current = true;
    setStreaming(true);
    requestBottomScroll("auto");
    segmentsRef.current = [];
    setStreamSegments([]);
    streamAccRef.current = "";
    currentStreamChatRef.current = capturedChatId;

    const isActive = () => currentStreamChatRef.current === capturedChatId;

    const flushSegments = () => {
      if (!isActive()) return;
      if (rafIdRef.current) return;
      rafIdRef.current = requestAnimationFrame(() => {
        rafIdRef.current = 0;
        if (isActive()) setStreamSegments(segmentsRef.current.map((s) => ({ ...s, toolCall: s.toolCall ? { ...s.toolCall } : undefined })));
      });
    };

    const appendText = (c: string) => {
      streamAccRef.current += c;
      const segs = segmentsRef.current;
      const last = segs[segs.length - 1];
      if (last && last.type === "text") {
        last.content = (last.content ?? "") + c;
      } else {
        segs.push({ id: `text-${segs.length}`, type: "text", content: c });
      }
    };

    let messageContent: string | unknown[];
    if (imageDataUrls.length > 0) {
      const parts: unknown[] = [];
      const textBody = txt + mentionDesc + fileDesc;
      if (textBody) parts.push({ type: "text", text: textBody });
      for (const img of imageDataUrls) {
        parts.push({ type: "image_url", image_url: { url: img.url } });
      }
      messageContent = parts;
    } else {
      messageContent = txt + mentionDesc + fileDesc;
    }

    const { promise: chatPromise, cleanup } = transport.chatStream(
      {
        messages: [{ role: "user", content: messageContent }],
        agentId: capturedAgentId,
        sessionId: capturedChatId,
        workDir: currentActiveChat?.workDir ?? undefined,
      },
      (event) => {
        switch (event.type) {
          case "chat.start": {
            streamAccRef.current = "";
            segmentsRef.current = [];
            const ds = detachedStreams.get(capturedChatId);
            if (ds) ds.acc = "";
            break;
          }
          case "chat.delta": {
            const c = event.data?.content as string | undefined;
            if (!c) return;
            if (isActive()) {
              appendText(c);
              flushSegments();
            } else {
              const ds = detachedStreams.get(capturedChatId);
              if (ds) ds.acc += c;
            }
            break;
          }
          case "chat.complete": {
            const sid = event.data?.sessionId as string | undefined;
            const ds = detachedStreams.get(capturedChatId);
            const finalContent = isActive() ? streamAccRef.current : ds?.acc ?? streamAccRef.current;
            const savedToolCalls = (isActive() ? segmentsRef.current : [])
              .filter((s) => s.type === "tool" && s.toolCall)
              .map((s) => {
                const tc = s.toolCall!;
                return { id: tc.id, name: tc.name, status: tc.status, args: tc.args, result: tc.result, duration: tc.duration };
              });

            if (isActive()) {
              cancelAnimationFrame(rafIdRef.current);
              rafIdRef.current = 0;
              streamAccRef.current = "";
              segmentsRef.current = [];
              currentStreamChatRef.current = null;
              setStreamSegments([]);
              setStreaming(false);
            }

            addMessage(capturedAgentId, {
              role: "assistant",
              content: finalContent,
              timestamp: new Date(),
              toolCalls: savedToolCalls.length > 0 ? savedToolCalls : undefined,
            }, capturedChatId);

            if (ds) {
              ds.done = true;
              detachedStreams.delete(capturedChatId);
            }
            cleanup();

            if (sid && capturedChatId !== sid) {
              updateChatBackendId(capturedAgentId, capturedChatId, sid);
            }

            // Track usage metrics (after ID remap so usage lands on the final chat ID)
            const usageData = event.data?.usage as { promptTokens?: number; completionTokens?: number; totalTokens?: number } | undefined;
            const elapsedMs = (event.data?.elapsedMs as number) ?? 0;
            const contextTokens = (event.data?.contextTokens as number) || undefined;
            const contextWindow = (event.data?.contextWindow as number) || undefined;
            if (usageData || elapsedMs || contextTokens) {
              const resolvedChatId = sid ?? capturedChatId;
              updateChatUsage(capturedAgentId, resolvedChatId, {
                promptTokens: usageData?.promptTokens ?? 0,
                completionTokens: usageData?.completionTokens ?? 0,
                totalTokens: usageData?.totalTokens ?? 0,
                elapsedMs,
                contextTokens,
                contextWindow,
              });
            }

            if (isActive() && atBottomRef.current) {
              requestBottomScroll("smooth");
            }
            break;
          }
          case "chat.tool.start": {
            const d = event.data;
            if (!d?.tool) return;
            const tc: ToolCall = {
              id: (d.callId ?? d.tool) as string,
              name: d.tool as string,
              status: "running",
              args: d.args as string | undefined,
              startTime: Date.now(),
            };
            if (isActive()) {
              segmentsRef.current.push({ id: `tool-${tc.id}`, type: "tool", toolCall: tc });
              flushSegments();
            } else {
              const ds = detachedStreams.get(capturedChatId);
              if (ds) ds.toolCalls = [...ds.toolCalls.filter((t) => t.id !== tc.id), tc];
            }
            break;
          }
          case "chat.tool.done": {
            const d = event.data;
            if (!d?.tool) return;
            const callId = (d.callId ?? d.tool) as string;
            if (isActive()) {
              const seg = segmentsRef.current.find((s) => s.type === "tool" && s.toolCall?.id === callId);
              if (seg?.toolCall) {
                seg.toolCall.status = d.success ? "success" : "error";
                seg.toolCall.result = d.output as string | undefined;
                seg.toolCall.duration = seg.toolCall.startTime ? Date.now() - seg.toolCall.startTime : undefined;
              }
              flushSegments();
            } else {
              const ds = detachedStreams.get(capturedChatId);
              if (ds) {
                ds.toolCalls = ds.toolCalls.map((t) =>
                  t.id === callId
                    ? { ...t, status: d.success ? "success" : "error", result: d.output as string | undefined, duration: t.startTime ? Date.now() - t.startTime : undefined }
                    : t,
                );
              }
            }
            break;
          }
          case "chat.ask_question": {
            const d = event.data;
            if (d?.requestId && d?.question && isActive()) {
              const timeoutSecs = (d.timeoutSecs as number) ?? 60;
              setPendingQuestion({
                requestId: d.requestId as string,
                question: d.question as string,
                options: (d.options as Array<{ id: string; label: string }>) ?? [],
                timeoutSecs,
                expiresAt: Date.now() + timeoutSecs * 1000,
                allowMultiple: d.allowMultiple as boolean | undefined,
              });
            }
            break;
          }
          case "chat.error": {
            const e = event.error?.message ?? "未知错误";
            if (isActive()) {
              cancelAnimationFrame(rafIdRef.current);
              rafIdRef.current = 0;
              streamAccRef.current = "";
              segmentsRef.current = [];
              currentStreamChatRef.current = null;
              setStreamSegments([]);
              setStreaming(false);
            }
            addMessage(capturedAgentId, { role: "system", content: `错误: ${e}`, timestamp: new Date() }, capturedChatId);
            const ds = detachedStreams.get(capturedChatId);
            if (ds) { ds.error = true; ds.done = true; detachedStreams.delete(capturedChatId); }
            cleanup();

            if (isActive() && atBottomRef.current) {
              requestBottomScroll("smooth");
            }
            break;
          }
        }
      },
    );

    cleanupRef.current = cleanup;

    chatPromise.catch(() => {
      if (isActive()) { setStreaming(false); }
      cleanup();
    });
  };

  const sendRef = useRef(sendWithContent);
  sendRef.current = sendWithContent;
  const streamingRef = useRef(streaming);
  streamingRef.current = streaming;

  const handleMentionSend = useCallback(
    (txt: string, _mentions: InlineMention[]) => {
      if (!txt.trim() || streamingRef.current) return;
      mentionInputRef.current?.clear();
      setAttachedFiles((prev) => {
        prev.forEach((f) => { if (f.previewUrl) URL.revokeObjectURL(f.previewUrl); });
        return [];
      });
      sendRef.current(txt.trim(), _mentions);
    },
    [],
  );

  const stopStream = useCallback(() => {
    if (cleanupRef.current) {
      cleanupRef.current();
      cleanupRef.current = null;
    }
    const content = streamAccRef.current;
    const savedTC = segmentsRef.current
      .filter((s) => s.type === "tool" && s.toolCall)
      .map((s) => {
        const tc = s.toolCall!;
        return { id: tc.id, name: tc.name, status: tc.status, args: tc.args, result: tc.result, duration: tc.duration };
      });
    cancelAnimationFrame(rafIdRef.current);
    rafIdRef.current = 0;
    streamAccRef.current = "";
    segmentsRef.current = [];
    setStreamSegments([]);
    if (content) {
      addMessage(activeAgentId, {
        role: "assistant",
        content,
        timestamp: new Date(),
        toolCalls: savedTC.length > 0 ? savedTC : undefined,
      }, currentStreamChatRef.current ?? undefined);
    }
    currentStreamChatRef.current = null;
    setStreaming(false);
  }, [activeAgentId, addMessage]);

  const handleNewTopic = useCallback(() => {
    if (streaming) return;
    newChat(activeAgentId, workDir ?? undefined);
  }, [streaming, newChat, activeAgentId, workDir]);

  const chatKey = `${activeAgentId}:${activeChat?.localKey ?? ac?.activeChatId ?? ""}`;
  const prevChatKey = useRef(chatKey);

  useEffect(() => {
    if (prevChatKey.current !== chatKey) {
      const prevKey = prevChatKey.current;
      if (virtuosoRef.current && atBottomRef.current) {
        scrollPositions.current[prevKey] = 0;
      }
      prevChatKey.current = chatKey;
      pendingRestoreScrollTopRef.current = scrollPositions.current[chatKey] ?? null;
    }
  }, [chatKey]);

  const handleScroll = useCallback((e: React.UIEvent<HTMLDivElement>) => {
    if (Date.now() < suppressScrollTrackingUntilRef.current) return;
    if (!e.nativeEvent.isTrusted) return;
    const top = (e.target as HTMLDivElement).scrollTop;
    if (atBottomRef.current) {
      scrollPositions.current[chatKey] = 0;
      return;
    }
    scrollPositions.current[chatKey] = top;
  }, [chatKey]);

  const PAGE_SIZE = 50;
  const [visibleCount, setVisibleCount] = useState(PAGE_SIZE);
  useEffect(() => {
    setVisibleCount(PAGE_SIZE);
  }, [chatKey]);

  const hasMore = stream.length > visibleCount;
  const paginationOffset = hasMore ? stream.length - visibleCount : 0;
  paginationOffsetRef.current = paginationOffset;
  const visibleStream = hasMore ? stream.slice(paginationOffset) : stream;

  const displayData = useMemo(() => {
    if (streaming) {
      return [
        ...visibleStream,
        { key: "_streaming_", data: { role: "streaming" as const, content: "", timestamp: new Date() } },
      ];
    }
    return visibleStream;
  }, [visibleStream, streaming]);

  useEffect(() => {
    if (pendingBottomScrollBehaviorRef.current == null || !virtuosoRef.current) return;
    const behavior = pendingBottomScrollBehaviorRef.current;
    pendingBottomScrollBehaviorRef.current = null;
    requestAnimationFrame(() => {
      runProgrammaticScroll(() => {
        virtuosoRef.current?.scrollToIndex({ index: "LAST", align: "end", behavior });
      });
    });
  }, [displayData.length, chatKey, runProgrammaticScroll]);

  useEffect(() => {
    if (pendingRestoreScrollTopRef.current == null || !virtuosoRef.current) return;
    const restoreTop = pendingRestoreScrollTopRef.current;
    pendingRestoreScrollTopRef.current = null;
    requestAnimationFrame(() => {
      requestAnimationFrame(() => {
        runProgrammaticScroll(() => {
          virtuosoRef.current?.scrollTo({ top: restoreTop });
        }, 360);
      });
    });
  }, [chatKey, displayData.length, runProgrammaticScroll]);

  const handleStartReached = useCallback(() => {
    if (hasMore) {
      let startIndex = 0;
      virtuosoRef.current?.getState((state) => {
        startIndex = state.ranges?.[0]?.startIndex ?? 0;
      });
      setVisibleCount((prev) => {
        const next = Math.min(prev + PAGE_SIZE, stream.length);
        const added = next - prev;
        if (added > 0) {
          requestAnimationFrame(() => {
            runProgrammaticScroll(() => {
              virtuosoRef.current?.scrollToIndex({
                index: startIndex + added,
                align: "start",
                behavior: "auto",
              });
            });
          });
        }
        return next;
      });
    }
  }, [hasMore, stream.length, runProgrammaticScroll]);

  useEffect(() => {
    if (searchResults.length > 0 && virtuosoRef.current) {
      const fullIdx = searchResults[searchIdx]?.idx;
      if (fullIdx != null) {
        const visibleIdx = fullIdx - paginationOffsetRef.current;
        if (visibleIdx < 0) {
          const neededVisibleCount = stream.length - fullIdx;
          setVisibleCount((prev) => Math.max(prev, neededVisibleCount));
          return;
        }
        if (visibleIdx >= 0 && visibleIdx < displayData.length) {
          runProgrammaticScroll(() => {
            virtuosoRef.current?.scrollToIndex({ index: visibleIdx, align: "center", behavior: "smooth" });
          });
        }
      }
    }
  }, [searchIdx, searchResults, displayData.length, stream.length, runProgrammaticScroll]);

  const isEmpty = stream.length === 0 && !streaming;

  return (
    <div
      className="relative flex min-h-0 flex-1 flex-col"
      style={{ background: "var(--bg-primary)" }}
      onDragEnter={handleDragEnter}
      onDragLeave={handleDragLeave}
      onDragOver={handleDragOver}
      onDrop={handleDrop}
    >
      {/* Drag overlay */}
      {isDragging && (
        <div
          className="absolute inset-0 z-30 flex items-center justify-center"
          style={{ background: "rgba(0, 122, 255, 0.06)", animation: "fade-in 0.15s" }}
        >
          <div
            className="flex flex-col items-center gap-3 rounded-[var(--radius-xl)] border-2 border-dashed px-12 py-10"
            style={{
              borderColor: "var(--tint)",
              background: "var(--bg-elevated)",
              boxShadow: "var(--shadow-lg)",
              animation: "drop-zone-pulse 1.5s ease-in-out infinite",
            }}
          >
            <Upload size={32} strokeWidth={1.5} style={{ color: "var(--fill-secondary)" }} />
            <span className="text-[15px] font-medium" style={{ color: "var(--fill-primary)" }}>
              拖拽文件到这里
            </span>
            <span className="text-[12px]" style={{ color: "var(--fill-tertiary)" }}>
              支持图片、文档、代码文件
            </span>
          </div>
        </div>
      )}

      {/* Agent Header */}
      <div
        className="vibrancy flex shrink-0 items-center justify-between px-6 py-3"
        style={{ background: "var(--bg-sidebar)", borderBottom: `0.5px solid var(--separator)` }}
      >
        <div className="flex min-w-0 flex-1 items-center gap-3">
          <div
            className="flex h-9 w-9 shrink-0 items-center justify-center rounded-full text-[13px] font-semibold"
            style={{ background: agent.color, color: "white" }}
          >
            {agent.initial}
          </div>
          <div className="min-w-0">
            <div className="truncate text-[14px] font-semibold" style={{ color: "var(--fill-primary)" }} title={agent.name}>{agent.name}</div>
            <div className="mt-0.5 flex items-center gap-1.5 text-[11px]" style={{ color: "var(--fill-tertiary)" }}>
              <span className="inline-block h-[6px] w-[6px] rounded-full" style={{ background: agent.online ? "var(--green)" : "var(--fill-quaternary)" }} />
              {agent.online ? "在线" : "离线"}
            </div>
          </div>
        </div>
        <div className="flex shrink-0 items-center gap-1">
          <button
            onClick={openSearch}
            className="flex h-8 w-8 items-center justify-center rounded-full transition-colors duration-100 hover:bg-[var(--bg-hover)]"
            style={{ color: searchOpen ? "var(--tint)" : "var(--fill-tertiary)" }}
            title="搜索消息 (⌘F)"
          >
            <Search size={15} strokeWidth={1.5} />
          </button>
          <button
            onClick={onToggleDetail}
            className="flex h-8 w-8 items-center justify-center rounded-full transition-colors duration-100 hover:bg-[var(--bg-hover)]"
            style={{ color: detailOpen ? "var(--fill-primary)" : "var(--fill-tertiary)" }}
            title={detailOpen ? "关闭详情" : "打开详情"}
          >
            <Settings2 size={16} strokeWidth={1.5} />
          </button>
        </div>
      </div>

      {/* Chat Tabs */}
      {ac && <ChatTabsBar
        agentId={activeAgentId}
        chats={ac.chatList}
        activeChatId={ac.activeChatId}
        streamingChatIds={streamingChatIds}
        onSelect={(id) => setActiveChat(activeAgentId, id)}
        onClose={(id) => closeChat(activeAgentId, id)}
        onNew={() => newChat(activeAgentId, workDir ?? undefined)}
        onRename={(id, t) => renameChat(activeAgentId, id, t)}
        onReorder={(from, to) => reorderChats(activeAgentId, from, to)}
      />}

      {/* Search Bar */}
      {searchOpen && (
        <div
          className="flex shrink-0 items-center gap-2 px-4 py-2"
          style={{ background: "var(--bg-secondary)", borderBottom: `0.5px solid var(--separator)`, animation: "slide-down 0.15s ease-out" }}
        >
          <Search size={14} strokeWidth={1.5} style={{ color: "var(--fill-tertiary)" }} />
          <input
            ref={searchInputRef}
            value={searchQuery}
            onChange={(e) => { setSearchQuery(e.target.value); setSearchIdx(0); }}
            onKeyDown={(e) => {
              if (e.key === "Escape") closeSearch();
              if (e.key === "Enter" && !e.shiftKey) setSearchIdx((i) => (i + 1) % Math.max(searchResults.length, 1));
              if (e.key === "Enter" && e.shiftKey) setSearchIdx((i) => (i - 1 + Math.max(searchResults.length, 1)) % Math.max(searchResults.length, 1));
            }}
            placeholder="搜索消息..."
            className="min-w-0 flex-1 bg-transparent text-[13px] outline-none"
            style={{ color: "var(--fill-primary)" }}
          />
          {searchQuery && (
            <span className="shrink-0 text-[11px] tabular-nums" style={{ color: "var(--fill-tertiary)" }}>
              {searchResults.length > 0 ? `${searchIdx + 1}/${searchResults.length}` : "无结果"}
            </span>
          )}
          <div className="flex items-center gap-0.5">
            <button
              onClick={() => setSearchIdx((i) => (i - 1 + Math.max(searchResults.length, 1)) % Math.max(searchResults.length, 1))}
              disabled={searchResults.length === 0}
              className="flex h-6 w-6 items-center justify-center rounded-md transition-colors duration-100 hover:bg-[var(--bg-hover)] disabled:opacity-30"
              style={{ color: "var(--fill-tertiary)" }}
            >
              <ChevronUp size={10} strokeWidth={2} />
            </button>
            <button
              onClick={() => setSearchIdx((i) => (i + 1) % Math.max(searchResults.length, 1))}
              disabled={searchResults.length === 0}
              className="flex h-6 w-6 items-center justify-center rounded-md transition-colors duration-100 hover:bg-[var(--bg-hover)] disabled:opacity-30"
              style={{ color: "var(--fill-tertiary)" }}
            >
              <ChevronDown size={10} strokeWidth={2} />
            </button>
          </div>
          <button
            onClick={closeSearch}
            className="flex h-6 w-6 items-center justify-center rounded-md transition-colors duration-100 hover:bg-[var(--bg-hover)]"
            style={{ color: "var(--fill-tertiary)" }}
          >
            <X size={10} strokeWidth={2} />
          </button>
        </div>
      )}

      {/* Messages */}
      {isEmpty ? (
        <div className="flex-1 overflow-y-auto px-8 py-6">
          <EmptyState onPick={(t) => {
            if (mentionInputRef.current) {
              mentionInputRef.current.clear();
              handleMentionSend(t, []);
            }
          }} />
        </div>
      ) : (
        <Virtuoso
          ref={virtuosoRef}
          key={chatKey}
          data={displayData}
          initialTopMostItemIndex={Math.max(0, displayData.length - 1)}
          followOutput={(isAtBottom) => (isAtBottom ? "smooth" : false)}
          atBottomStateChange={handleAtBottomChange}
          atBottomThreshold={120}
          className="flex-1"
          style={{ overflowX: "hidden", overflowY: "scroll" }}
          onScroll={handleScroll}
          startReached={handleStartReached}
          itemContent={(idx, item) => {
            const m = item.data;

            if (m.role === "streaming") {
              const hasContent = streamSegments.length > 0;
              const lastSeg = streamSegments[streamSegments.length - 1];
              const lastIsText = lastSeg?.type === "text";
              return (
                <div className="px-8 pb-4">
                  {!hasContent && <Typing />}
                  {streamSegments.map((seg, si) => {
                    if (seg.type === "text" && seg.content) {
                      const isLast = si === streamSegments.length - 1;
                      return (
                        <div key={seg.id} className="pb-1" style={{ maxWidth: "75%" }}>
                          <MarkdownContent content={seg.content} />
                          {isLast && (
                            <span
                              className="ml-0.5 inline-block h-[16px] w-[2px] translate-y-[3px] rounded-full"
                              style={{ background: "var(--tint)", animation: "cursor-blink 1s step-end infinite" }}
                            />
                          )}
                        </div>
                      );
                    }
                    if (seg.type === "tool" && seg.toolCall) {
                      return <ToolCallCard key={seg.id} tool={seg.toolCall} />;
                    }
                    return null;
                  })}
                  {hasContent && !lastIsText && (
                    <div className="mt-1"><Typing /></div>
                  )}
                  <div ref={bottomRef} />
                </div>
              );
            }

            const fullIdx = idx + paginationOffset;
            const isMatch = searchQuery && m.content.toLowerCase().includes(searchQuery.toLowerCase());
            const isCurrent = isMatch && searchResults[searchIdx]?.idx === fullIdx;
            return (
              <div
                className="px-8 transition-colors duration-200"
                style={{
                  background: isCurrent ? "var(--tint-bg)" : isMatch ? "var(--tint-subtle)" : "transparent",
                }}
              >
                {m.role === "user" ? <UserBubble msg={m} /> :
                 m.role === "system" ? <SystemMsg msg={m} /> :
                 <AiMessage msg={m} usage={!streaming && idx === visibleStream.length - 1 && m.role === "assistant" ? activeChat?.usage : undefined} />}
              </div>
            );
          }}
          increaseViewportBy={{ top: 200, bottom: 200 }}
          components={{
            Header: () => (
              <div className="flex h-8 items-center justify-center">
                {hasMore && (
                  <span className="text-[11px]" style={{ color: "var(--fill-quaternary)" }}>
                    ↑ 滚动加载更多
                  </span>
                )}
              </div>
            ),
            Footer: () => <div className="h-8" />,
          }}
        />
      )}

      {/* ─── Large Input Area ─── */}
      <div className="relative shrink-0 px-6 pb-5 pt-2">
        {/* ─── Question Panel (ask_question tool) ─── */}
        {pendingQuestion && (
          <QuestionPanel
            question={pendingQuestion}
            onAnswer={async (answer) => {
              setPendingQuestion(null);
              await transport.submitToolAnswerIpc(pendingQuestion.requestId, answer);
            }}
            onTimeout={() => setPendingQuestion(null)}
          />
        )}

        <div
          className="overflow-hidden rounded-2xl transition-shadow duration-200"
          style={{
            background: "var(--bg-elevated)",
            border: `1px solid var(--separator)`,
            boxShadow: "var(--shadow-md)",
          }}
        >
          {/* File attachments */}
          {attachedFiles.length > 0 && (
            <div className="flex flex-wrap gap-1.5 px-4 pt-3" style={{ animation: "slide-up 0.15s ease-out" }}>
              {attachedFiles.map((f, i) => (
                <FilePill key={`${f.name}-${i}`} file={f} onRemove={() => removeFile(i)} />
              ))}
            </div>
          )}

          <MentionInput
            ref={mentionInputRef}
            disabled={streaming}
            placeholder="描述任务，或输入 @ 引用文件、目录、Skill..."
            options={mentionOptions}
            onSend={handleMentionSend}
            onNewTopic={handleNewTopic}
            onAttach={() => fileInputRef.current?.click()}
            onPasteFiles={processFiles}
          />

          {/* Bottom action bar */}
          <div className="flex items-center justify-between gap-2 px-3.5 pb-3">
            <div className="flex min-w-0 items-center gap-0.5">
              <button
                onClick={() => fileInputRef.current?.click()}
                className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg transition-colors duration-100 hover:bg-[var(--bg-hover)]"
                style={{ color: "var(--fill-tertiary)" }}
                title="附件 (⌘⇧A)"
              >
                <Paperclip size={16} strokeWidth={1.5} />
              </button>
              <button
                onClick={onToggleDetail}
                className="flex shrink-0 cursor-pointer items-center gap-1 rounded-lg px-2 py-1 text-[12px] font-medium transition-colors duration-100 hover:bg-[var(--bg-hover)]"
                style={{ color: detailOpen ? "var(--fill-secondary)" : "var(--fill-tertiary)" }}
                title="工具"
              >
                <Settings2 size={14} strokeWidth={1.5} />
                工具
              </button>

              <div className="mx-1 h-4 w-px shrink-0" style={{ background: "var(--separator)" }} />

              {/* WorkDir selector */}
              <button
                onClick={async () => {
                  const currentState = useAgentStore.getState();
                  const curAgentId = currentState.activeAgentId;
                  const curAc = currentState.agentChats[curAgentId];
                  const curChat = curAc?.chatList.find((c) => c.id === curAc.activeChatId);
                  if (!curChat) return;
                  let selected: string | null = null;
                  try {
                    selected = await tauriOpenDialog({ directory: true, multiple: false, defaultPath: curChat.workDir ?? undefined }) as string | null;
                  } catch {
                    selected = prompt("输入工作目录路径:", curChat.workDir ?? "");
                  }
                  if (typeof selected === "string" && selected) {
                    setWorkDir(curAgentId, curChat.id, selected);
                  }
                }}
                className="flex min-w-0 items-center gap-1.5 rounded-lg px-2 py-1 text-[12px] transition-colors duration-100 hover:bg-[var(--bg-hover)]"
                style={{ color: workDir ? "var(--fill-secondary)" : "var(--fill-quaternary)" }}
                title={workDir ? `工作目录: ${workDir}` : "设置工作目录"}
              >
                <FolderOpen className="shrink-0" size={13} strokeWidth={1.5} />
                <span className="max-w-[120px] truncate font-mono text-[11px]">
                  {workDir ? workDir.replace(/^\/home\/[^/]+\//, "~/") : "工作目录"}
                </span>
              </button>

              {!detailOpen && (
                <>
                  <div className="mx-1 h-4 w-px shrink-0" style={{ background: "var(--separator)" }} />
                  <ShortcutsHint />
                </>
              )}
            </div>

            <div className="flex shrink-0 items-center gap-2">
              {activeChat?.usage?.contextTokens != null && activeChat?.usage?.contextWindow != null && activeChat.usage.contextWindow > 0 && (
                <ContextRing
                  used={activeChat.usage.contextTokens}
                  limit={activeChat.usage.contextWindow}
                />
              )}
              {streaming ? (
                <button
                  key="stop"
                  onClick={stopStream}
                  className="flex h-8 w-8 shrink-0 cursor-pointer items-center justify-center rounded-full transition-colors duration-150"
                  style={{
                    background: "var(--fill-warning, #ED8936)",
                    color: "#fff",
                  }}
                  title="停止生成"
                >
                  <Square size={12} strokeWidth={2.5} fill="currentColor" />
                </button>
              ) : (
                <button
                  key="send"
                  onClick={() => {
                    const ref = mentionInputRef.current;
                    if (ref) {
                      const t = ref.getText().trim();
                      if (t) handleMentionSend(t, ref.getMentions());
                    }
                  }}
                  className="flex h-8 w-8 shrink-0 cursor-pointer items-center justify-center rounded-full transition-colors duration-150 disabled:opacity-25"
                  style={{
                    background: "var(--fill-primary)",
                    color: "var(--fill-inverse)",
                  }}
                  title="发送 ↩"
                >
                  <ArrowUp size={16} strokeWidth={2} />
                </button>
              )}
            </div>
          </div>
        </div>

        {/* Hidden file input */}
        <input
          ref={fileInputRef}
          type="file"
          multiple
          className="hidden"
          onChange={(e) => { if (e.target.files) processFiles(e.target.files); e.target.value = ""; }}
        />
      </div>
    </div>
  );
}
