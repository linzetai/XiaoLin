import { Component, memo, useMemo, useState, useRef, useCallback, useEffect, lazy, Suspense, type ReactNode, type ErrorInfo } from "react";
import type { ChatMessage, ChatUsage, SubAgentRunUI } from "../../lib/agent-store";
import { ToolCallCard } from "./ToolCallCard";
import { SubAgentCard } from "./SubAgentCard";
import {
  groupConsecutiveSegments,
  groupConsecutiveToolCalls,
  ToolCallGroupCard,
  ToolCallGroupTimeline,
} from "./ToolCallGroup";
import { AlertTriangle } from "lucide-react";

const MarkdownContent = lazy(() =>
  import("./MarkdownContent").then((m) => ({ default: m.MarkdownContent })),
);

class MessageErrorBoundary extends Component<
  { children: ReactNode },
  { error: Error | null }
> {
  state: { error: Error | null } = { error: null };

  static getDerivedStateFromError(error: Error) {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    console.error("[MessageErrorBoundary]", error, info.componentStack);
  }

  render() {
    if (this.state.error) {
      return (
        <div
          className="mx-6 my-2 flex items-center gap-2 rounded-lg px-3 py-2 text-[12px]"
          style={{
            background: "color-mix(in srgb, var(--red) 6%, transparent)",
            border: "0.5px solid color-mix(in srgb, var(--red) 20%, transparent)",
            color: "var(--red)",
          }}
        >
          <AlertTriangle size={14} strokeWidth={1.5} />
          <span>渲染出错：{this.state.error.message}</span>
          <button
            onClick={() => this.setState({ error: null })}
            className="ml-auto cursor-pointer text-[11px] font-medium underline"
            style={{ color: "var(--fill-tertiary)" }}
          >
            重试
          </button>
        </div>
      );
    }
    return this.props.children;
  }
}
import {
  Clock, ArrowUpRight, ArrowDownRight, Copy, Check,
} from "lucide-react";
import type { StreamSegment } from "./types";
import { useConfigStore } from "../../lib/stores/config-store";

function ts(d: Date) {
  return d.toLocaleTimeString("zh-CN", { hour: "2-digit", minute: "2-digit" });
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

const UserBubble = memo(function UserBubble({ msg, copyable, selected, onToggleSelect, animate = true }: { msg: ChatMessage; copyable?: boolean; selected?: boolean; onToggleSelect?: () => void; animate?: boolean }) {
  const { text, tags } = useMemo(() => parseUserContent(msg.content), [msg.content]);
  const [copied, setCopied] = useState(false);
  const handleCopy = useCallback(() => {
    navigator.clipboard.writeText(text).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    });
  }, [text]);
  return (
    <div className="pb-5 flex justify-end group/message" style={{ animation: animate ? "slide-right var(--duration-normal) var(--ease-out)" : "none" }}>
      <div className="flex flex-col items-end" style={{ maxWidth: "65%" }}>
        <div className="flex items-start gap-2">
          {onToggleSelect && (
            <button
              onClick={onToggleSelect}
              className="mt-3 flex h-4 w-4 shrink-0 items-center justify-center rounded border transition-colors duration-100 hover:border-[var(--fill-secondary)]"
              style={{
                borderColor: selected ? "var(--tint)" : "var(--fill-quaternary)",
                background: selected ? "var(--tint)" : "transparent",
              }}
            >
              {selected && <Check size={10} strokeWidth={2.5} style={{ color: "white" }} />}
            </button>
          )}
          <div
            className="user-bubble-content rounded-2xl px-4 py-3 text-[15px] leading-[1.6] break-words relative"
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
          {copyable && (
            <button
              onClick={handleCopy}
              className="mt-2 flex h-6 w-6 shrink-0 items-center justify-center rounded-md transition-colors duration-100 hover:bg-[var(--bg-hover)] opacity-0 group-hover/message:opacity-100"
              style={{ color: "var(--fill-tertiary)" }}
              title="复制"
            >
              {copied ? <Check size={12} strokeWidth={2} /> : <Copy size={12} strokeWidth={1.5} />}
            </button>
          )}
        </div>
        <span className="mt-1 pr-1 text-[10px]" style={{ color: "var(--fill-quaternary)" }}>
          {ts(msg.timestamp)}
        </span>
      </div>
    </div>
  );
});

const AiMessage = memo(function AiMessage({ msg, usage, copyable, selected, onToggleSelect, animate = true }: { msg: ChatMessage; usage?: ChatUsage; copyable?: boolean; selected?: boolean; onToggleSelect?: () => void; animate?: boolean }) {
  const toolCalls = msg.toolCalls;
  const [copied, setCopied] = useState(false);
  const handleCopy = useCallback(() => {
    navigator.clipboard.writeText(msg.content).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    });
  }, [msg.content]);
  const aiThreshold = useConfigStore((s) => s.display.toolCallGroupThreshold);
  const groupedToolCalls = useMemo(() => {
    if (!toolCalls || toolCalls.length === 0) return null;
    const typed = toolCalls.map((tc) => ({ ...tc, status: tc.status as "running" | "success" | "error" }));
    return groupConsecutiveToolCalls(typed, aiThreshold);
  }, [toolCalls, aiThreshold]);
  return (
    <div className="pb-7 group/message" style={{ animation: animate ? "slide-left var(--duration-normal) var(--ease-out)" : "none", maxWidth: "75%" }}>
      <div className="flex items-start gap-2">
        {onToggleSelect && (
          <button
            onClick={onToggleSelect}
            className="mt-1 flex h-4 w-4 shrink-0 items-center justify-center rounded border transition-colors duration-100 hover:border-[var(--fill-secondary)]"
            style={{
              borderColor: selected ? "var(--tint)" : "var(--fill-quaternary)",
              background: selected ? "var(--tint)" : "transparent",
            }}
          >
            {selected && <Check size={10} strokeWidth={2.5} style={{ color: "white" }} />}
          </button>
        )}
        <div className="flex-1 min-w-0">
      {groupedToolCalls && groupedToolCalls.length > 0 && (
        <div className="mb-2">
          {groupedToolCalls.map((item) => {
            if (item.type === "single") {
              return <ToolCallCard key={item.tool.id} tool={item.tool} />;
            }
            return (
              <ToolCallGroupCard
                key={item.tools[0].id}
                tools={item.tools}
              />
            );
          })}
        </div>
      )}
      <Suspense fallback={<div className="animate-pulse rounded py-2" style={{ background: "var(--bg-tertiary)", height: 20 }} />}>
        <MarkdownContent content={msg.content} />
      </Suspense>
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
        {copyable && (
          <button
            onClick={handleCopy}
            className="mt-1 flex h-6 w-6 shrink-0 items-center justify-center rounded-md transition-colors duration-100 hover:bg-[var(--bg-hover)] opacity-0 group-hover/message:opacity-100"
            style={{ color: "var(--fill-tertiary)" }}
            title="复制"
          >
            {copied ? <Check size={12} strokeWidth={2} /> : <Copy size={12} strokeWidth={1.5} />}
          </button>
        )}
      </div>
    </div>
  );
});

function SystemMsg({ msg }: { msg: ChatMessage }) {
  const isError = typeof msg.content === "string" && msg.content.startsWith("错误:");
  return (
    <div
      className="pb-4 break-words rounded-xl px-3 py-2 text-[13px]"
      style={{
        background: isError ? "var(--error-subtle, rgba(239,68,68,0.08))" : "var(--tint-subtle)",
        color: isError ? "var(--error-text, #dc2626)" : "var(--fill-secondary)",
        borderLeft: isError ? "3px solid var(--error-border, #f87171)" : "none",
        overflowWrap: "anywhere",
      }}
    >
      {msg.content}
    </div>
  );
}

const OPTION_LETTERS = "ABCDEFGHIJKLMNOPQRSTUVWXYZ";

export function QuestionPanel({
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
  const hasTimeout = question.timeoutSecs > 0 && question.expiresAt > 0;
  const [remaining, setRemaining] = useState(() => hasTimeout ? Math.max(0, Math.ceil((question.expiresAt - Date.now()) / 1000)) : 0);
  const [freeText, setFreeText] = useState("");
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [submitted, setSubmitted] = useState(false);
  const panelRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!hasTimeout) return;
    const timer = setInterval(() => {
      const left = Math.max(0, Math.ceil((question.expiresAt - Date.now()) / 1000));
      setRemaining(left);
      if (left <= 0) {
        clearInterval(timer);
        onTimeout();
      }
    }, 200);
    return () => clearInterval(timer);
  }, [hasTimeout, question.expiresAt, onTimeout]);

  const progress = hasTimeout ? Math.max(0, remaining / question.timeoutSecs) : 1;
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
        animation: reducedMotion ? "none" : "slide-up var(--duration-normal) var(--ease-out)",
      }}
    >
      {hasTimeout && (
        <div className="relative h-[2px] w-full" style={{ background: "var(--bg-tertiary)" }}>
          <div
            className="absolute inset-y-0 left-0 transition-all duration-200"
            style={{ width: `${progress * 100}%`, background: remaining <= 10 ? "var(--fill-warning, #ED8936)" : "var(--fill-accent, #4299E1)" }}
          />
        </div>
      )}
      <div className="px-4 py-3">
        <div className="mb-3 flex items-center justify-between gap-2">
          <p className="text-[13px] font-medium" style={{ color: "var(--fill-primary)" }}>{question.question}</p>
          {hasTimeout && (
            <span className="shrink-0 text-[11px] tabular-nums" style={{ color: remaining <= 10 ? "var(--fill-warning, #ED8936)" : "var(--fill-tertiary)" }}>
              {remaining}s
            </span>
          )}
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

function Typing() {
  return (
    <div className="pb-6 flex items-center gap-1" style={{ animation: "fade-in var(--duration-fast)" }}>
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

type StreamableMsg = ChatMessage | { role: "streaming"; content: string; timestamp: Date };

export interface MessageRendererRowProps {
  item: { data: StreamableMsg };
  idx: number;
  paginationOffset: number;
  searchQuery: string;
  searchIdx: number;
  searchResults: Array<{ item: { data: ChatMessage }; idx: number }>;
  streamSegments: StreamSegment[];
  subAgentRuns: Record<string, SubAgentRunUI> | undefined;
  bottomRef: React.RefObject<HTMLDivElement | null>;
  selectMode?: boolean;
  isSelected?: boolean;
  onToggleSelect?: (fullIdx: number) => void;
  animate?: boolean;
}

export function MessageRendererRow({
  item,
  idx,
  paginationOffset,
  searchQuery,
  searchIdx,
  searchResults,
  streamSegments,
  subAgentRuns,
  bottomRef,
  selectMode,
  isSelected,
  onToggleSelect,
  animate = true,
}: MessageRendererRowProps) {
  const m = item.data as StreamableMsg;
  const threshold = useConfigStore((s) => s.display.toolCallGroupThreshold);
  const grouped = useMemo(() => groupConsecutiveSegments(streamSegments, threshold), [streamSegments, threshold]);

  if (m.role === "streaming") {
    const hasContent = streamSegments.length > 0;
    const lastSeg = streamSegments[streamSegments.length - 1];
    const lastIsText = lastSeg?.type === "text";
    const activeSubRuns = subAgentRuns ? Object.values(subAgentRuns) : [];
    return (
      <MessageErrorBoundary>
      <div className="px-8 pb-4">
        {!hasContent && activeSubRuns.length === 0 && <Typing />}
        {grouped.map((group, gi) => {
          if (group.type === "text" && group.segment.content) {
            const isLastSegment = gi === grouped.length - 1 && lastIsText;
            return (
              <div key={group.segment.id} className="pb-1" style={{ maxWidth: "75%" }}>
                <Suspense fallback={<div className="animate-pulse rounded py-1" style={{ background: "var(--bg-tertiary)", height: 16 }} />}>
                  <MarkdownContent content={group.segment.content} streaming />
                </Suspense>
                {isLastSegment && (
                  <span
                    className="ml-0.5 inline-block h-[16px] w-[2px] translate-y-[3px] rounded-full"
                    style={{ background: "var(--tint)", animation: "cursor-blink 1s step-end infinite" }}
                  />
                )}
              </div>
            );
          }
          if (group.type === "single-tool" && group.segment.toolCall) {
            return <ToolCallCard key={group.segment.id} tool={group.segment.toolCall} />;
          }
          if (group.type === "tool-group") {
            const tools = group.segments
              .map((s) => s.toolCall)
              .filter((tc): tc is NonNullable<typeof tc> => !!tc);
            return (
              <ToolCallGroupTimeline
                key={group.segments[0].id}
                tools={tools}
              />
            );
          }
          return null;
        })}
        {activeSubRuns.length > 0 && (
          <div className="mt-1">
            {activeSubRuns.map((run) => (
              <SubAgentCard key={run.runId} run={run} />
            ))}
          </div>
        )}
        {hasContent && !lastIsText && activeSubRuns.length === 0 && (
          <div className="mt-1"><Typing /></div>
        )}
        <div ref={bottomRef} />
      </div>
      </MessageErrorBoundary>
    );
  }

  const cm = m as ChatMessage;
  const fullIdx = idx + paginationOffset;
  const isMatch = searchQuery && cm.content.toLowerCase().includes(searchQuery.toLowerCase());
  const isCurrent = isMatch && searchResults[searchIdx]?.idx === fullIdx;
  return (
    <MessageErrorBoundary>
    <div
      className="px-8 transition-colors duration-200"
      style={{
        background: isCurrent ? "var(--tint-bg)" : isMatch ? "var(--tint-subtle)" : "transparent",
      }}
    >
      {cm.role === "user" ? (
        <UserBubble
          msg={cm}
          copyable
          selected={selectMode ? isSelected : undefined}
          onToggleSelect={selectMode ? () => onToggleSelect?.(fullIdx) : undefined}
          animate={animate}
        />
      ) : cm.role === "system" ? (
        <SystemMsg msg={cm} />
      ) : (
        <AiMessage
          msg={cm}
          usage={cm.usage}
          copyable
          selected={selectMode ? isSelected : undefined}
          onToggleSelect={selectMode ? () => onToggleSelect?.(fullIdx) : undefined}
          animate={animate}
        />
      )}
    </div>
    </MessageErrorBoundary>
  );
}
