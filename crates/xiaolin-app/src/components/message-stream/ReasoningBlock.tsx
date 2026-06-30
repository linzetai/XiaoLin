import { useState, useEffect, useRef, useCallback, useMemo } from "react";
import { useTranslation } from "react-i18next";

const LEAKED_TAG_RE =
  /<\/?(?:mcp_instructions|mcp_server_instructions|user_provided_context|session_guidance|session_memory|code_context|goal_context|untrusted_objective|security|memory|system|system_prompt|instructions|tool_instructions)(?:\s[^>]*)?\s*\/?>/gi;

function sanitizeReasoning(raw: string): string {
  return raw.replace(LEAKED_TAG_RE, "").replace(/^\s*---\s*\n/, "").trimStart();
}

interface ReasoningBlockProps {
  content: string;
  isStreaming?: boolean;
  autoCollapse?: boolean;
}

export function ReasoningBlock({ content, isStreaming, autoCollapse }: ReasoningBlockProps) {
  const { t } = useTranslation("chat");
  const [expanded, setExpanded] = useState(false);
  const contentRef = useRef<HTMLDivElement>(null);
  const userScrolledUp = useRef(false);

  useEffect(() => {
    if (autoCollapse) {
      setExpanded(false);
    }
  }, [autoCollapse]);

  const handleScroll = useCallback(() => {
    const el = contentRef.current;
    if (!el) return;
    const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 8;
    userScrolledUp.current = !atBottom;
  }, []);

  useEffect(() => {
    if (!isStreaming || !expanded) return;
    const el = contentRef.current;
    if (!el || userScrolledUp.current) return;
    el.scrollTop = el.scrollHeight;
  }, [content, isStreaming, expanded]);

  const cleaned = useMemo(() => sanitizeReasoning(content), [content]);

  const isActive = isStreaming && !autoCollapse;

  const startTime = useRef<number | null>(null);
  const [elapsed, setElapsed] = useState(0);
  const frozenElapsed = useRef<number | null>(null);

  useEffect(() => {
    if (isActive) {
      if (startTime.current === null) startTime.current = Date.now();
      const timer = setInterval(() => {
        setElapsed((Date.now() - startTime.current!) / 1000);
      }, 500);
      return () => clearInterval(timer);
    }
    if (startTime.current !== null && frozenElapsed.current === null) {
      frozenElapsed.current = (Date.now() - startTime.current) / 1000;
      setElapsed(frozenElapsed.current);
    }
  }, [isActive]);

  const displayTime = isActive
    ? `${elapsed.toFixed(1)}s`
    : frozenElapsed.current !== null
      ? `${frozenElapsed.current.toFixed(1)}s`
      : null;

  return (
    <div
      className="my-1.5 pl-3 relative"
      style={{
        borderLeft: `2px solid ${isActive ? "color-mix(in srgb, var(--tint) 55%, var(--fill-quaternary))" : "color-mix(in srgb, var(--fill-quaternary) 35%, transparent)"}`,
        transition: "border-color 300ms ease-out",
      }}
    >
      {/* Pulsing dot at top-left */}
      {isActive && (
        <span
          className="absolute -left-[5px] top-[6px] block h-[8px] w-[8px] rounded-full"
          style={{
            background: "color-mix(in srgb, var(--tint) 60%, var(--fill-quaternary))",
            animation: "reasoning-pulse 1.5s ease-in-out infinite",
          }}
        />
      )}

      {/* Header */}
      <button
        type="button"
        className="flex items-center gap-1.5 text-left text-[12px] cursor-pointer select-none py-0.5"
        style={{ color: "var(--fill-tertiary)" }}
        onClick={() => setExpanded((v) => !v)}
      >
        <svg
          width={10}
          height={10}
          viewBox="0 0 10 10"
          fill="none"
          className="shrink-0 transition-transform duration-200"
          style={{ transform: expanded ? "rotate(90deg)" : "rotate(0deg)" }}
        >
          <path d="M3.5 2L7 5L3.5 8" stroke="currentColor" strokeWidth={1.2} strokeLinecap="round" strokeLinejoin="round" />
        </svg>
        <span className="opacity-70">
          {isActive ? t("reasoning_streaming") : t("reasoning_done")}
        </span>
        {displayTime && <span className="ml-1 tabular-nums opacity-50">{displayTime}</span>}
      </button>

      {/* Content panel with animated height */}
      <div
        style={{
          maxHeight: expanded ? (isStreaming ? 200 : 9999) : 0,
          overflow: "hidden",
          transition: "max-height 300ms ease-out",
        }}
      >
        <div
          ref={contentRef}
          onScroll={handleScroll}
          className="text-[11.5px] leading-relaxed whitespace-pre-wrap break-words pr-1"
          style={{
            color: "var(--fill-tertiary)",
            fontFamily: "var(--font-mono, monospace)",
            maxHeight: isStreaming ? 200 : "none",
            overflowY: isStreaming ? "auto" : "visible",
            opacity: 0.75,
          }}
        >
          {cleaned}
        </div>
      </div>
    </div>
  );
}
