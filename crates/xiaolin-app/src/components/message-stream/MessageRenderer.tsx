import { Component, memo, useMemo, useState, useRef, useCallback, useEffect, lazy, Suspense, type ReactNode, type ErrorInfo } from "react";
import type { ChatMessage, ChatUsage, SubAgentRunUI } from "../../lib/agent-store";
import type { BriefMessageData } from "../../lib/stores/types";
import { ICON, ICON_ACTIVE_STROKE, BTN_ICON } from "../../lib/ui-tokens";
import { StepIndicator } from "./StepIndicator";
import { SubAgentCard } from "./SubAgentCard";
import {
  groupConsecutiveSegments,
  groupConsecutiveToolCalls,
  StepGroup,
} from "./StepGroup";
import { AlertTriangle } from "lucide-react";
import { UserInput } from "./UserInput";
import { BriefMessageCard } from "./BriefMessageCard";
import { ClawIcon } from "../layout/ClawIcon";

const MarkdownContent = lazy(() =>
  import("./MarkdownContent").then((m) => ({ default: m.MarkdownContent })),
);
import { StreamingMarkdown } from "./StreamingMarkdown";

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
          <AlertTriangle {...ICON.sm} />
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
  Clock, Copy, Check, ThumbsUp, ThumbsDown, RotateCw,
} from "lucide-react";
import type { StreamSegment } from "./types";
import { useConfigStore } from "../../lib/stores/config-store";

function MessageAvatar({ role }: { role: "user" | "assistant" }) {
  if (role === "user") {
    return (
      <div
        className="w-[30px] h-[30px] rounded-full grid place-items-center text-[12px] font-bold text-white shrink-0"
        style={{ background: "linear-gradient(135deg, var(--tint), color-mix(in srgb, var(--tint) 70%, #6366F1))" }}
      >
        U
      </div>
    );
  }
  return (
    <div
      className="w-[30px] h-[30px] rounded-full grid place-items-center shrink-0"
      style={{ background: "var(--bg-surface)", border: "1.5px solid var(--separator)" }}
    >
      <ClawIcon size={14} />
    </div>
  );
}

function ts(d: Date) {
  const now = new Date();
  const isToday =
    d.getFullYear() === now.getFullYear() &&
    d.getMonth() === now.getMonth() &&
    d.getDate() === now.getDate();
  if (isToday) {
    return d.toLocaleTimeString("zh-CN", { hour: "2-digit", minute: "2-digit" });
  }
  return d.toLocaleDateString("zh-CN", { month: "2-digit", day: "2-digit" }) +
    " " +
    d.toLocaleTimeString("zh-CN", { hour: "2-digit", minute: "2-digit" });
}

const AiReactionBar = memo(function AiReactionBar({ content, sessionId, turnId }: { content: string; sessionId?: string; turnId?: string }) {
  const [copied, setCopied] = useState(false);
  const [liked, setLiked] = useState(false);
  const [disliked, setDisliked] = useState(false);

  const handleCopy = useCallback(() => {
    navigator.clipboard.writeText(content).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    });
  }, [content]);

  const handleLike = useCallback(() => {
    const next = !liked;
    setLiked(next);
    if (disliked) setDisliked(false);
    if (next && sessionId && turnId) {
      import("../../lib/transport").then((t) => t.submitFeedback(sessionId, turnId, "positive").catch(() => {}));
    }
  }, [liked, disliked, sessionId, turnId]);

  const handleDislike = useCallback(() => {
    const next = !disliked;
    setDisliked(next);
    if (liked) setLiked(false);
    if (next && sessionId && turnId) {
      import("../../lib/transport").then((t) => t.submitFeedback(sessionId, turnId, "negative").catch(() => {}));
    }
  }, [liked, disliked, sessionId, turnId]);

  const btnCls = `${BTN_ICON.sm} transition-all duration-150 active:scale-90`;
  const iconProps = { ...ICON.sm, strokeWidth: 1.5 } as const;
  const defaultColor = "var(--fill-tertiary)";

  return (
    <div
      className="mt-1 flex items-center gap-0.5 -ml-1.5 opacity-0 group-hover/message:opacity-100 transition-opacity duration-150"
    >
      <button onClick={handleCopy} className={btnCls} style={{ color: copied ? "var(--green)" : defaultColor }} title="复制">
        {copied ? <Check {...iconProps} strokeWidth={ICON_ACTIVE_STROKE} style={{ animation: "scale-spring var(--duration-normal) var(--ease-spring)" }} /> : <Copy {...iconProps} />}
      </button>
      <button
        onClick={handleLike}
        className={btnCls}
        style={{ color: liked ? "var(--tint)" : defaultColor }}
        title="点赞"
      >
        <ThumbsUp {...iconProps} strokeWidth={liked ? ICON_ACTIVE_STROKE : iconProps.strokeWidth} />
      </button>
      <button
        onClick={handleDislike}
        className={btnCls}
        style={{ color: disliked ? "var(--red)" : defaultColor }}
        title="点踩"
      >
        <ThumbsDown {...iconProps} strokeWidth={disliked ? ICON_ACTIVE_STROKE : iconProps.strokeWidth} />
      </button>
      <button
        onClick={() => {
          if (sessionId && turnId) {
            import("../../lib/transport").then((t) => t.retryTurn(sessionId, turnId).catch(() => {}));
          }
        }}
        className={btnCls}
        style={{ color: defaultColor }}
        title="重试"
      >
        <RotateCw {...iconProps} />
      </button>
    </div>
  );
});

const AiMessage = memo(function AiMessage({ msg, usage, copyable, selected, onToggleSelect, savedSegments }: { msg: ChatMessage; usage?: ChatUsage; copyable?: boolean; selected?: boolean; onToggleSelect?: () => void; savedSegments?: StreamSegment[] }) {
  const toolCalls = msg.toolCalls;
  const aiThreshold = useConfigStore((s) => s.display.toolCallGroupThreshold);

  const hasSegments = savedSegments && savedSegments.length > 0;
  const groupedSegments = useMemo(() => {
    if (!hasSegments) return null;
    return groupConsecutiveSegments(savedSegments!, aiThreshold);
  }, [hasSegments, savedSegments, aiThreshold]);

  const groupedToolCalls = useMemo(() => {
    if (hasSegments) return null;
    if (!toolCalls || toolCalls.length === 0) return null;
    const typed = toolCalls.map((tc) => ({ ...tc, status: tc.status as "running" | "success" | "error" }));
    return groupConsecutiveToolCalls(typed, aiThreshold);
  }, [hasSegments, toolCalls, aiThreshold]);

  return (
    <div className="pb-3 group/message">
      <div className="flex items-start gap-[14px]">
        {onToggleSelect && (
          <button
            onClick={onToggleSelect}
            className="mt-1 flex h-4 w-4 shrink-0 items-center justify-center rounded border transition-colors duration-100 hover:border-[var(--fill-secondary)]"
            style={{
              borderColor: selected ? "var(--tint)" : "var(--fill-quaternary)",
              background: selected ? "var(--tint)" : "transparent",
            }}
          >
            {selected && <Check size={14} strokeWidth={2.5} style={{ color: "white" }} />}
          </button>
        )}
        <MessageAvatar role="assistant" />
        <div className="flex-1 min-w-0">
      <div className="flex items-center gap-2 mb-1">
        <span className="text-[13px] font-semibold" style={{ color: "var(--fill-primary)" }}>XiaoLin</span>
        <span className="text-[11px] tabular-nums" style={{ color: "var(--fill-quaternary)" }}>
          {ts(msg.timestamp)}
        </span>
        {usage && (
          <span
            className="inline-flex items-center gap-0.5 rounded-full px-1.5 py-0.5 text-[10.5px]"
            style={{ background: "var(--bg-secondary)", color: "var(--fill-quaternary)" }}
            title={`上行 ${formatTokens(usage.promptTokens)} · 下行 ${formatTokens(usage.completionTokens)}`}
          >
            <Clock size={10} strokeWidth={1.2} />
            {formatElapsed(usage.elapsedMs)}
          </span>
        )}
      </div>
      {groupedSegments ? (
        <div className="mb-2" style={{ maxWidth: "var(--content-max-w)" }}>
          {groupedSegments.map((group) => {
            if (group.type === "text" && group.segment.content) {
              return (
                <div key={group.segment.id} className="pb-1">
                  <Suspense fallback={<div className="animate-pulse rounded py-1" style={{ background: "var(--bg-tertiary)", height: 16 }} />}>
                    <MarkdownContent content={group.segment.content} />
                  </Suspense>
                </div>
              );
            }
            if (group.type === "single-tool" && group.segment.toolCall) {
              return <StepIndicator key={group.segment.id} tool={group.segment.toolCall} />;
            }
            if (group.type === "tool-group") {
              const tools = group.segments
                .map((s) => s.toolCall)
                .filter((tc): tc is NonNullable<typeof tc> => !!tc);
              return <StepGroup key={group.segments[0].id} tools={tools} />;
            }
            return null;
          })}
        </div>
      ) : (
        <>
          {groupedToolCalls && groupedToolCalls.length > 0 && (
            <div className="mb-2" style={{ maxWidth: "var(--content-max-w)" }}>
              {groupedToolCalls.map((item) => {
                if (item.type === "single") {
                  return <StepIndicator key={item.tool.id} tool={item.tool} />;
                }
                return (
                  <StepGroup
                    key={item.tools[0].id}
                    tools={item.tools}
                  />
                );
              })}
            </div>
          )}
          <div style={{ maxWidth: "var(--content-max-w)" }}>
            <Suspense fallback={<div className="animate-pulse rounded py-2" style={{ background: "var(--bg-tertiary)", height: 20 }} />}>
              <MarkdownContent content={msg.content} />
            </Suspense>
          </div>
        </>
      )}
      {!groupedSegments && copyable && <AiReactionBar content={msg.content} sessionId={msg.chatId} turnId={String(msg.id)} />}
      {groupedSegments && copyable && <AiReactionBar content={msg.content} sessionId={msg.chatId} turnId={String(msg.id)} />}
        </div>
      </div>
    </div>
  );
});

function SystemMsg({ msg }: { msg: ChatMessage }) {
  const isError = typeof msg.content === "string" && msg.content.startsWith("错误:");
  return (
    <div
      className="pb-2 flex items-start gap-2 text-[13px]"
      style={{
        color: isError ? "var(--red)" : "var(--fill-tertiary)",
        overflowWrap: "anywhere",
      }}
    >
      <span
        className="mt-[7px] inline-block h-[6px] w-[6px] shrink-0 rounded-full"
        style={{ background: isError ? "var(--red)" : "var(--tint)" }}
      />
      <span className="break-words min-w-0">{msg.content}</span>
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
    <div className="pb-6 flex items-center gap-1">
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

type DisplayItem =
  | { type?: "message"; data: StreamableMsg }
  | { type: "brief"; data: BriefMessageData };

export interface MessageRendererRowProps {
  item: DisplayItem;
  idx: number;
  paginationOffset: number;
  searchQuery: string;
  searchIdx: number;
  searchResults: Array<{ item: DisplayItem; idx: number }>;
  streamSegments: StreamSegment[];
  subAgentRuns: Record<string, SubAgentRunUI> | undefined;
  bottomRef: React.RefObject<HTMLDivElement | null>;
  selectMode?: boolean;
  isSelected?: boolean;
  onToggleSelect?: (fullIdx: number) => void;
  lastSegments?: StreamSegment[];
}

export const MessageRendererRow = memo(function MessageRendererRow({
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
  lastSegments,
}: MessageRendererRowProps) {
  if (item.type === "brief") {
    return <BriefMessageCard data={item.data as BriefMessageData} />;
  }

  const m = item.data as StreamableMsg;
  const threshold = useConfigStore((s) => s.display.toolCallGroupThreshold);
  const grouped = useMemo(() => groupConsecutiveSegments(streamSegments, threshold), [streamSegments, threshold]);

  const isStreaming = m.role === "streaming";
  const cm = (isStreaming ? m : m) as ChatMessage;
  const fullIdx = idx + paginationOffset;
  const isMatch = !isStreaming && searchQuery && cm.content?.toLowerCase().includes(searchQuery.toLowerCase());
  const isCurrent = isMatch && searchResults[searchIdx]?.idx === fullIdx;
  const rowRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (isStreaming) return;
    const el = rowRef.current;
    if (!el) return;
    const existing = el.querySelectorAll("mark[data-search-highlight]");
    existing.forEach((mark) => {
      const parent = mark.parentNode;
      if (parent) {
        parent.replaceChild(document.createTextNode(mark.textContent ?? ""), mark);
        parent.normalize();
      }
    });

    if (!searchQuery || !isMatch) return;

    const q = searchQuery.toLowerCase();
    const walker = document.createTreeWalker(el, NodeFilter.SHOW_TEXT);
    const textNodes: Text[] = [];
    let node: Text | null;
    while ((node = walker.nextNode() as Text | null)) {
      if (node.textContent && node.textContent.toLowerCase().includes(q)) {
        textNodes.push(node);
      }
    }

    for (const textNode of textNodes) {
      const text = textNode.textContent ?? "";
      const lower = text.toLowerCase();
      const parts: (string | { match: string })[] = [];
      let lastIdx = 0;
      let pos = lower.indexOf(q);
      while (pos !== -1) {
        if (pos > lastIdx) parts.push(text.slice(lastIdx, pos));
        parts.push({ match: text.slice(pos, pos + q.length) });
        lastIdx = pos + q.length;
        pos = lower.indexOf(q, lastIdx);
      }
      if (lastIdx < text.length) parts.push(text.slice(lastIdx));
      if (!parts.some((p) => typeof p !== "string")) continue;

      const frag = document.createDocumentFragment();
      for (const part of parts) {
        if (typeof part === "string") {
          frag.appendChild(document.createTextNode(part));
        } else {
          const mark = document.createElement("mark");
          mark.setAttribute("data-search-highlight", isCurrent ? "current" : "");
          mark.textContent = part.match;
          frag.appendChild(mark);
        }
      }
      textNode.parentNode?.replaceChild(frag, textNode);
    }
  }, [isStreaming, searchQuery, isMatch, isCurrent]);

  if (isStreaming) {
    const hasContent = streamSegments.length > 0;
    const lastSeg = streamSegments[streamSegments.length - 1];
    const lastIsText = lastSeg?.type === "text";
    const activeSubRuns = subAgentRuns ? Object.values(subAgentRuns) : [];
    return (
      <MessageErrorBoundary>
      <div className="px-6 pb-2">
        <div style={{ maxWidth: "var(--content-max-w)" }}>
        {!hasContent && activeSubRuns.length === 0 && <Typing />}
        {grouped.map((group, gi) => {
          if (group.type === "text" && group.segment.content) {
            const isLastSegment = gi === grouped.length - 1 && lastIsText;
            return (
              <div key={group.segment.id} className="pb-1">
                <StreamingMarkdown content={group.segment.content} />
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
            return <StepIndicator key={group.segment.id} tool={group.segment.toolCall} />;
          }
          if (group.type === "tool-group") {
            const tools = group.segments
              .map((s) => s.toolCall)
              .filter((tc): tc is NonNullable<typeof tc> => !!tc);
            return (
              <StepGroup
                key={group.segments[0].id}
                tools={tools}
                streaming
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
      </div>
      </MessageErrorBoundary>
    );
  }

  return (
    <MessageErrorBoundary>
    <div
      ref={rowRef}
      className="px-6"
    >
      {cm.role === "user" ? (
        <UserInput
          msg={cm}
          copyable
          selected={selectMode ? isSelected : undefined}
          onToggleSelect={selectMode ? () => onToggleSelect?.(fullIdx) : undefined}
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
          savedSegments={lastSegments}
        />
      )}
    </div>
    </MessageErrorBoundary>
  );
}, (prev, next) => {
  const prevMsg = prev.item.data as StreamableMsg;
  const nextMsg = next.item.data as StreamableMsg;
  if (prevMsg.role === "streaming" || nextMsg.role === "streaming") return false;
  return (
    prev.item === next.item &&
    prev.searchQuery === next.searchQuery &&
    prev.searchIdx === next.searchIdx &&
    prev.lastSegments === next.lastSegments &&
    prev.selectMode === next.selectMode &&
    prev.isSelected === next.isSelected
  );
});
