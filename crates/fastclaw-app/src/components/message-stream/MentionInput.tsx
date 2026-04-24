import {
  useState,
  useRef,
  useCallback,
  useEffect,
  useMemo,
  forwardRef,
  useImperativeHandle,
  type KeyboardEvent,
} from "react";
import { createPortal } from "react-dom";
import { File, Folder, Sparkles } from "lucide-react";

/* ─── Types ─── */
export type MentionType = "file" | "dir" | "skill";

export interface MentionOption {
  id: string;
  label: string;
  type: MentionType;
  desc?: string;
}

export interface InlineMention {
  type: MentionType;
  id: string;
  label: string;
  start: number;
  end: number;
}

export interface MentionInputHandle {
  focus: () => void;
  clear: () => void;
  getText: () => string;
  setText: (value: string) => void;
  getMentions: () => InlineMention[];
}

interface MentionInputProps {
  disabled?: boolean;
  placeholder?: string;
  options: MentionOption[];
  onSend: (text: string, mentions: InlineMention[]) => void;
  onNewTopic: () => void;
  onAttach: () => void;
  onPasteFiles?: (files: File[]) => void;
  extraKeyHandler?: (e: KeyboardEvent<HTMLTextAreaElement>) => boolean;
}

const MENTION_TYPE_META: Record<MentionType, { text: string; icon: React.ReactNode; color: string }> = {
  file: { text: "文件", icon: <File size={12} strokeWidth={1.5} />, color: "var(--tint)" },
  dir: { text: "目录", icon: <Folder size={12} strokeWidth={1.5} />, color: "var(--orange)" },
  skill: { text: "Skill", icon: <Sparkles size={12} strokeWidth={1.5} />, color: "var(--green)" },
};

/* ─── Mention Popup ─── */
function MentionPopup({
  options,
  selectedIndex,
  onSelect,
  rect,
}: {
  options: MentionOption[];
  selectedIndex: number;
  onSelect: (opt: MentionOption) => void;
  rect: { top: number; left: number; width: number } | null;
}) {
  if (options.length === 0 || !rect) return null;

  let lastType: MentionType | null = null;
  const popupWidth = Math.min(300, window.innerWidth - 48);

  return (
    <div
      style={{
        position: "fixed",
        zIndex: 9999,
        bottom: window.innerHeight - rect.top + 8,
        left: rect.left,
        width: popupWidth,
        maxHeight: 320,
        overflowY: "auto",
        background: "var(--bg-elevated)",
        border: "0.5px solid var(--separator)",
        boxShadow: "var(--shadow-lg)",
        borderRadius: "var(--radius-sm)",
        animation: "slide-up 0.15s ease-out",
      }}
    >
      <div className="py-1">
        {options.map((opt, i) => {
          const showHeader = opt.type !== lastType;
          lastType = opt.type;
          const meta = MENTION_TYPE_META[opt.type];

          return (
            <div key={opt.id}>
              {showHeader && (
                <div className="px-3 pt-2 pb-1">
                  <span
                    className="text-[10px] font-semibold uppercase tracking-wider"
                    style={{ color: "var(--fill-tertiary)" }}
                  >
                    {meta.text}
                  </span>
                </div>
              )}
              <button
                onClick={() => onSelect(opt)}
                className="flex w-full cursor-pointer items-center gap-2.5 px-3 py-2 text-left text-[13px] transition-colors duration-75 hover:bg-[var(--tint-bg)]"
                style={{
                  background: i === selectedIndex ? "var(--tint-bg)" : "transparent",
                  color: "var(--fill-primary)",
                }}
              >
                <span
                  className="flex h-6 w-6 shrink-0 items-center justify-center rounded text-[12px]"
                  style={{ background: `color-mix(in srgb, ${meta.color} 8%, transparent)`, color: meta.color }}
                >
                  {meta.icon}
                </span>
                <div className="min-w-0 flex-1">
                  <div className="flex min-w-0 items-center gap-1.5">
                    <span className="min-w-0 truncate font-medium">{opt.label}</span>
                  </div>
                  {opt.desc && (
                    <div
                      className="mt-0.5 truncate text-[11px]"
                      style={{ color: "var(--fill-tertiary)" }}
                    >
                      {opt.desc}
                    </div>
                  )}
                </div>
              </button>
            </div>
          );
        })}
      </div>
    </div>
  );
}

/* ─── Highlight Overlay ─── */
function HighlightOverlay({ text, mentions }: { text: string; mentions: InlineMention[] }) {
  if (mentions.length === 0) {
    return <span>{text || "\u00A0"}</span>;
  }

  const sorted = [...mentions].sort((a, b) => a.start - b.start);
  const parts: React.ReactNode[] = [];
  let cursor = 0;

  for (const m of sorted) {
    if (m.start > cursor) {
      parts.push(<span key={`t-${cursor}`}>{text.slice(cursor, m.start)}</span>);
    }
    const meta = MENTION_TYPE_META[m.type];
    parts.push(
      <span
        key={`m-${m.start}`}
        className="mention-chip"
        style={{
          background: `color-mix(in srgb, ${meta.color} 8%, transparent)`,
          color: meta.color,
          borderColor: `color-mix(in srgb, ${meta.color} 20%, transparent)`,
        }}
      >
        @{m.label}
      </span>,
    );
    cursor = m.end;
  }

  if (cursor < text.length) {
    parts.push(<span key={`t-${cursor}`}>{text.slice(cursor)}</span>);
  }

  return <>{parts}</>;
}

/* ─── MentionInput ─── */
export const MentionInput = forwardRef<MentionInputHandle, MentionInputProps>(
  function MentionInput(
    { disabled, placeholder, options, onSend, onNewTopic, onAttach, onPasteFiles, extraKeyHandler },
    ref,
  ) {
    const [text, setText] = useState("");
    const [mentions, setMentions] = useState<InlineMention[]>([]);

    const [popupActive, setPopupActive] = useState(false);
    const [popupQuery, setPopupQuery] = useState("");
    const [popupIndex, setPopupIndex] = useState(0);
    const [popupStart, setPopupStart] = useState(-1);

    const taRef = useRef<HTMLTextAreaElement>(null);
    const containerRef = useRef<HTMLDivElement>(null);

    useImperativeHandle(ref, () => ({
      focus: () => taRef.current?.focus(),
      clear: () => {
        setText("");
        setMentions([]);
        if (taRef.current) taRef.current.style.height = "auto";
      },
      getText: () => text,
      setText: (value: string) => {
        setText(value);
        setMentions([]);
      },
      getMentions: () => mentions,
    }));

    const filteredOptions = useMemo(
      () =>
        options.filter(
          (o) =>
            !popupQuery ||
            o.label.toLowerCase().includes(popupQuery.toLowerCase()) ||
            o.desc?.toLowerCase().includes(popupQuery.toLowerCase()),
        ),
      [options, popupQuery],
    );

    const updateMentionPositions = useCallback(
      (oldText: string, newText: string, cursorPos: number) => {
        const diff = newText.length - oldText.length;
        if (diff === 0) return mentions;

        return mentions
          .map((m) => {
            if (cursorPos <= m.start) {
              return { ...m, start: m.start + diff, end: m.end + diff };
            }
            if (cursorPos > m.start && cursorPos <= m.end) {
              return null;
            }
            return m;
          })
          .filter((m): m is InlineMention => m !== null);
      },
      [mentions],
    );

    const handleChange = useCallback(
      (e: React.ChangeEvent<HTMLTextAreaElement>) => {
        const newVal = e.target.value;
        const cursorPos = e.target.selectionStart ?? 0;

        const updatedMentions = updateMentionPositions(text, newVal, cursorPos);
        const validMentions = updatedMentions.filter((m) => {
          if (m.start < 0 || m.end > newVal.length) return false;
          const expected = `@${m.label}`;
          const actual = newVal.slice(m.start, m.end);
          return actual === expected;
        });
        setMentions(validMentions);
        setText(newVal);

        const beforeCursor = newVal.slice(0, cursorPos);
        const atMatch = beforeCursor.match(/@([^\s@]*)$/);
        if (atMatch) {
          const isInsideMention = validMentions.some(
            (m) => cursorPos > m.start && cursorPos <= m.end,
          );
          if (!isInsideMention) {
            setPopupActive(true);
            setPopupQuery(atMatch[1]);
            setPopupStart(cursorPos - atMatch[0].length);
            setPopupIndex(0);
            return;
          }
        }
        setPopupActive(false);
        setPopupQuery("");
        setPopupStart(-1);
      },
      [text, updateMentionPositions],
    );

    const insertMention = useCallback(
      (opt: MentionOption) => {
        if (popupStart < 0 || !taRef.current) return;

        const cursorPos = taRef.current.selectionStart ?? text.length;
        const before = text.slice(0, popupStart);
        const after = text.slice(cursorPos);
        const mentionText = `@${opt.label}`;
        const newText = `${before}${mentionText} ${after}`;

        const newMention: InlineMention = {
          type: opt.type,
          id: opt.id,
          label: opt.label,
          start: popupStart,
          end: popupStart + mentionText.length,
        };

        const updatedMentions = mentions
          .map((m) => {
            const shift = newText.length - text.length;
            if (m.start >= popupStart) {
              return { ...m, start: m.start + shift, end: m.end + shift };
            }
            return m;
          })
          .concat(newMention);

        setText(newText);
        setMentions(updatedMentions);
        setPopupActive(false);
        setPopupQuery("");
        setPopupStart(-1);

        const newCursorPos = popupStart + mentionText.length + 1;
        setTimeout(() => {
          if (taRef.current) {
            taRef.current.setSelectionRange(newCursorPos, newCursorPos);
            taRef.current.focus();
          }
        }, 0);
      },
      [text, mentions, popupStart],
    );

    const handleKeyDown = useCallback(
      (e: KeyboardEvent<HTMLTextAreaElement>) => {
        if (extraKeyHandler?.(e)) return;

        const isMod = e.metaKey || e.ctrlKey;

        if (popupActive && filteredOptions.length > 0) {
          if (e.key === "ArrowDown") {
            e.preventDefault();
            setPopupIndex((i) => (i + 1) % filteredOptions.length);
            return;
          }
          if (e.key === "ArrowUp") {
            e.preventDefault();
            setPopupIndex((i) => (i - 1 + filteredOptions.length) % filteredOptions.length);
            return;
          }
          if (e.key === "Enter" || e.key === "Tab") {
            e.preventDefault();
            insertMention(filteredOptions[popupIndex]);
            return;
          }
          if (e.key === "Escape") {
            e.preventDefault();
            setPopupActive(false);
            return;
          }
        }

        if (e.key === "Backspace" && taRef.current && mentions.length > 0) {
          const cursorPos = taRef.current.selectionStart ?? 0;
          const selEnd = taRef.current.selectionEnd ?? cursorPos;

          if (cursorPos === selEnd) {
            const mentionAtCursor = mentions.find((m) => cursorPos === m.end);
            if (mentionAtCursor) {
              e.preventDefault();
              const before = text.slice(0, mentionAtCursor.start);
              const after = text.slice(mentionAtCursor.end);
              const newText = before + after;
              const removedLen = mentionAtCursor.end - mentionAtCursor.start;

              const updatedMentions = mentions
                .filter((m) => m !== mentionAtCursor)
                .map((m) => {
                  if (m.start > mentionAtCursor.start) {
                    return { ...m, start: m.start - removedLen, end: m.end - removedLen };
                  }
                  return m;
                });

              setText(newText);
              setMentions(updatedMentions);

              setTimeout(() => {
                if (taRef.current) {
                  taRef.current.setSelectionRange(mentionAtCursor.start, mentionAtCursor.start);
                }
              }, 0);
              return;
            }
          }
        }

        if (e.key === "Enter" && !e.shiftKey && !popupActive) {
          e.preventDefault();
          if (text.trim()) onSend(text, mentions);
          return;
        }

        if (isMod && e.key === "k") {
          e.preventDefault();
          onNewTopic();
          return;
        }

        if (isMod && e.shiftKey && e.key === "A") {
          e.preventDefault();
          onAttach();
          return;
        }
      },
      [
        text,
        mentions,
        popupActive,
        filteredOptions,
        popupIndex,
        insertMention,
        onSend,
        onNewTopic,
        onAttach,
        extraKeyHandler,
      ],
    );

    const MAX_HEIGHT = 160;

    const autoGrow = useCallback(() => {
      const el = taRef.current;
      if (!el) return;
      el.style.height = "auto";
      const clamped = Math.min(el.scrollHeight, MAX_HEIGHT);
      el.style.height = clamped + "px";
      el.style.overflowY = el.scrollHeight > MAX_HEIGHT ? "auto" : "hidden";
    }, []);

    const syncOverlayScroll = useCallback(() => {
      const ta = taRef.current;
      const overlay = ta?.parentElement?.querySelector<HTMLElement>(".mention-highlight-overlay");
      if (ta && overlay) overlay.scrollTop = ta.scrollTop;
    }, []);

    useEffect(() => {
      autoGrow();
    }, [text, autoGrow]);

    const [popupRect, setPopupRect] = useState<{ top: number; left: number; width: number } | null>(null);

    useEffect(() => {
      if (!popupActive || !containerRef.current) {
        setPopupRect(null);
        return;
      }
      const r = containerRef.current.getBoundingClientRect();
      setPopupRect({ top: r.top, left: r.left, width: r.width });
    }, [popupActive, text]);

    const popupEl = popupActive ? (
      <MentionPopup
        options={filteredOptions}
        selectedIndex={popupIndex}
        onSelect={insertMention}
        rect={popupRect}
      />
    ) : null;

    return (
      <div ref={containerRef} className="mention-input-container">
        {popupEl && createPortal(popupEl, document.body)}

        <div className="mention-input-wrapper">
          <div className="mention-highlight-overlay" aria-hidden="true">
            <HighlightOverlay text={text} mentions={mentions} />
          </div>
          <textarea
            ref={taRef}
            value={text}
            onChange={handleChange}
            onKeyDown={handleKeyDown}
            onInput={autoGrow}
            onScroll={syncOverlayScroll}
            onPaste={(e) => {
              const items = e.clipboardData?.items;
              if (!items || !onPasteFiles) return;
              const imageFiles: File[] = [];
              for (const item of items) {
                if (item.type.startsWith("image/")) {
                  const file = item.getAsFile();
                  if (file) imageFiles.push(file);
                }
              }
              if (imageFiles.length > 0) {
                e.preventDefault();
                onPasteFiles(imageFiles);
              }
            }}
            placeholder={placeholder}
            rows={2}
            disabled={disabled}
            className="mention-textarea"
            spellCheck={false}
          />
        </div>
      </div>
    );
  },
);
