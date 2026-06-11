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
import { useTranslation } from "react-i18next";
import { createPortal } from "react-dom";
import {
  useFloating,
  offset,
  flip,
  shift,
  autoUpdate,
  size as floatingSize,
} from "@floating-ui/react";
import { File, Folder, Sparkle, MagnifyingGlass, Terminal } from "@phosphor-icons/react";
import { fuzzyFilter, type FuzzyResult } from "../../lib/fuzzy";

/* ─── Types ─── */
export type MentionType = "file" | "dir" | "skill";
export type TriggerType = "@" | "/";

export interface MentionOption {
  id: string;
  label: string;
  type: MentionType;
  desc?: string;
}

export interface SlashCommand {
  id: string;
  label: string;
  desc: string;
  action?: () => void;
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
  slashCommands?: SlashCommand[];
  onSend: (text: string, mentions: InlineMention[]) => void;
  onNewTopic: () => void;
  onAttach: () => void;
  onPasteFiles?: (files: File[]) => void;
  onRecallLastMessage?: () => string | null;
  onContentChange?: (hasContent: boolean) => void;
  extraKeyHandler?: (e: KeyboardEvent<HTMLTextAreaElement>) => boolean;
}

function useMentionTypeMeta() {
  const { t } = useTranslation("chat");
  return useMemo(
    (): Record<MentionType, { text: string; icon: React.ReactNode; color: string }> => ({
      file: { text: t("mention_file"), icon: <File />, color: "var(--tint)" },
      dir: { text: t("mention_dir"), icon: <Folder />, color: "var(--orange)" },
      skill: { text: "Skill", icon: <Sparkle />, color: "var(--green)" },
    }),
    [t],
  );
}

/* ─── Fuzzy Highlight ─── */
function FuzzyHighlight({ text, indices }: { text: string; indices: number[] }) {
  if (indices.length === 0) return <span>{text}</span>;

  const indexSet = new Set(indices);
  const parts: React.ReactNode[] = [];
  let i = 0;

  while (i < text.length) {
    if (indexSet.has(i)) {
      let end = i;
      while (end < text.length && indexSet.has(end)) end++;
      parts.push(
        <span key={i} style={{ color: "var(--tint)", fontWeight: 600 }}>
          {text.slice(i, end)}
        </span>,
      );
      i = end;
    } else {
      let end = i;
      while (end < text.length && !indexSet.has(end)) end++;
      parts.push(<span key={i}>{text.slice(i, end)}</span>);
      i = end;
    }
  }

  return <>{parts}</>;
}

/* ─── Smart Path Truncation ─── */
function truncatePath(path: string, maxLen = 32): string {
  if (path.length <= maxLen) return path;
  const parts = path.split("/");
  if (parts.length <= 2) return `…${path.slice(-(maxLen - 1))}`;
  const first = parts[0];
  const last = parts[parts.length - 1];
  const mid = parts.length - 2;
  return `${first}/…${mid > 1 ? `(${mid})` : ""}/${last}`;
}

/* ─── Popup Item Types ─── */
type PopupItem =
  | { kind: "mention"; option: MentionOption; result: FuzzyResult }
  | { kind: "command"; command: SlashCommand; result: FuzzyResult };

/* ─── Mention Popup ─── */
function MentionPopup({
  items,
  selectedIndex,
  onSelect,
  triggerType,
  query,
  floatingRef,
  floatingStyles,
}: {
  items: PopupItem[];
  selectedIndex: number;
  onSelect: (index: number) => void;
  triggerType: TriggerType;
  query: string;
  floatingRef: (node: HTMLElement | null) => void;
  floatingStyles: React.CSSProperties;
}) {
  const { t } = useTranslation("chat");
  const mentionTypeMeta = useMentionTypeMeta();
  const itemRefs = useRef<(HTMLButtonElement | null)[]>([]);

  useEffect(() => {
    const el = itemRefs.current[selectedIndex];
    if (el) el.scrollIntoView({ block: "nearest", behavior: "smooth" });
  }, [selectedIndex]);

  if (items.length === 0) {
    return (
      <div
        ref={floatingRef}
        style={{
          ...floatingStyles,
          zIndex: 9999,
          width: "min(300px, calc(100vw - 48px))",
          background: "var(--bg-elevated)",
          border: "0.5px solid var(--separator)",
          boxShadow: "var(--shadow-lg)",
          borderRadius: "var(--radius-sm)",
          animation: "slide-up var(--duration-fast) var(--ease-out)",
        }}
      >
        <div className="flex items-center gap-2 px-3 py-3">
          <MagnifyingGlass style={{ color: "var(--fill-quaternary)" }} />
          <span className="text-[12px]" style={{ color: "var(--fill-tertiary)" }}>
            {query
              ? t("mention_noResults", {
                  query,
                  type: triggerType === "/" ? t("mention_type_command") : t("mention_type_item"),
                })
              : t("mention_searchHint", {
                  hint: triggerType === "/" ? t("mention_hint_command") : t("mention_hint_filename"),
                })}
          </span>
        </div>
      </div>
    );
  }

  let lastGroup: string | null = null;

  return (
    <div
      ref={floatingRef}
      style={{
        ...floatingStyles,
        zIndex: 9999,
        width: "min(300px, calc(100vw - 48px))",
        maxHeight: 320,
        overflowY: "auto",
        background: "var(--bg-elevated)",
        border: "0.5px solid var(--border-subtle)",
        boxShadow: "var(--shadow-lg), inset 0 1px 0 var(--highlight-top)",
        borderRadius: "var(--radius-sm)",
        animation: "scale-spring var(--duration-fast) var(--ease-spring-subtle)",
        backdropFilter: "blur(12px)",
        WebkitBackdropFilter: "blur(12px)",
        transformOrigin: "bottom left",
      }}
    >
      <div className="py-1">
        {items.map((item, i) => {
          const group = getItemGroup(item, mentionTypeMeta, t);
          const showHeader = group !== lastGroup;
          lastGroup = group;
          const { icon, color } = getItemMeta(item, mentionTypeMeta);

          return (
            <div key={getItemKey(item)}>
              {showHeader && (
                <div className="px-3 pt-2 pb-1">
                  <span
                    className="text-[10px] font-semibold uppercase tracking-wider"
                    style={{ color: "var(--fill-tertiary)" }}
                  >
                    {group}
                  </span>
                </div>
              )}
              <button
                ref={(el) => { itemRefs.current[i] = el; }}
                onClick={() => onSelect(i)}
                className="flex w-full cursor-pointer items-center gap-2.5 px-3 py-2 text-left text-[13px] transition-colors duration-75 hover:bg-[var(--tint-bg)]"
                style={{
                  background: i === selectedIndex ? "var(--tint-bg)" : "transparent",
                  color: "var(--fill-primary)",
                }}
              >
                <span
                  className="flex h-6 w-6 shrink-0 items-center justify-center rounded text-[12px]"
                  style={{ background: `color-mix(in srgb, ${color} 8%, transparent)`, color }}
                >
                  {icon}
                </span>
                <div className="min-w-0 flex-1">
                  <div className="flex min-w-0 items-center gap-1.5">
                    <span className="min-w-0 truncate font-medium">
                      <FuzzyHighlight text={getItemLabel(item)} indices={item.result.indices} />
                    </span>
                  </div>
                  {getItemDesc(item) && (
                    <div
                      className="mt-0.5 truncate text-[11px]"
                      style={{ color: "var(--fill-tertiary)" }}
                    >
                      {getItemDesc(item)}
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

function getItemGroup(
  item: PopupItem,
  mentionTypeMeta: Record<MentionType, { text: string }>,
  t: (key: string) => string,
): string {
  if (item.kind === "mention") return mentionTypeMeta[item.option.type].text;
  return t("mention_commands");
}

function getItemMeta(
  item: PopupItem,
  mentionTypeMeta: Record<MentionType, { icon: React.ReactNode; color: string }>,
): { icon: React.ReactNode; color: string } {
  if (item.kind === "mention") return mentionTypeMeta[item.option.type];
  return { icon: <Terminal />, color: "var(--purple)" };
}

function getItemKey(item: PopupItem): string {
  if (item.kind === "mention") return `m-${item.option.id}`;
  return `c-${item.command.id}`;
}

function getItemLabel(item: PopupItem): string {
  if (item.kind === "mention") return item.option.label;
  return item.command.label;
}

function getItemDesc(item: PopupItem): string | undefined {
  if (item.kind === "mention") return item.option.desc;
  return item.command.desc;
}

/* ─── Highlight Overlay ─── */
function HighlightOverlay({ text, mentions }: { text: string; mentions: InlineMention[] }) {
  const mentionTypeMeta = useMentionTypeMeta();
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
    const meta = mentionTypeMeta[m.type];
    parts.push(
      <span
        key={`m-${m.start}`}
        className="mention-chip"
        style={{
          background: `color-mix(in srgb, ${meta.color} 8%, transparent)`,
          color: meta.color,
          borderColor: `color-mix(in srgb, ${meta.color} 20%, transparent)`,
        }}
        title={m.label}
      >
        <span className="mention-chip-icon">{meta.icon}</span>
        <span className="mention-chip-label">{truncatePath(m.label)}</span>
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
    { disabled, placeholder, options, slashCommands, onSend, onNewTopic, onAttach, onPasteFiles, onRecallLastMessage, onContentChange, extraKeyHandler },
    ref,
  ) {
    const [text, setText] = useState("");
    const [mentions, setMentions] = useState<InlineMention[]>([]);

    const [popupActive, setPopupActive] = useState(false);
    const [popupQuery, setPopupQuery] = useState("");
    const [popupIndex, setPopupIndex] = useState(0);
    const [popupStart, setPopupStart] = useState(-1);
    const [triggerType, setTriggerType] = useState<TriggerType>("@");

    const taRef = useRef<HTMLTextAreaElement>(null);
    const containerRef = useRef<HTMLDivElement>(null);

    type HistoryEntry = { text: string; mentions: InlineMention[]; cursor: number };
    const historyRef = useRef<HistoryEntry[]>([{ text: "", mentions: [], cursor: 0 }]);
    const historyIdxRef = useRef(0);
    const pushHistory = useCallback((t: string, m: InlineMention[], cursor: number) => {
      const h = historyRef.current;
      const idx = historyIdxRef.current;
      if (h[idx]?.text === t) return;
      historyRef.current = [...h.slice(0, idx + 1), { text: t, mentions: m, cursor }];
      historyIdxRef.current = historyRef.current.length - 1;
      if (historyRef.current.length > 100) {
        historyRef.current = historyRef.current.slice(-80);
        historyIdxRef.current = historyRef.current.length - 1;
      }
    }, []);
    const undo = useCallback(() => {
      if (historyIdxRef.current <= 0) return;
      historyIdxRef.current--;
      const entry = historyRef.current[historyIdxRef.current];
      setText(entry.text);
      setMentions(entry.mentions);
      onContentChange?.(!!entry.text.trim());
      setTimeout(() => {
        taRef.current?.setSelectionRange(entry.cursor, entry.cursor);
      }, 0);
    }, [onContentChange]);
    const redo = useCallback(() => {
      if (historyIdxRef.current >= historyRef.current.length - 1) return;
      historyIdxRef.current++;
      const entry = historyRef.current[historyIdxRef.current];
      setText(entry.text);
      setMentions(entry.mentions);
      onContentChange?.(!!entry.text.trim());
      setTimeout(() => {
        taRef.current?.setSelectionRange(entry.cursor, entry.cursor);
      }, 0);
    }, [onContentChange]);

    const { refs, floatingStyles } = useFloating({
      open: popupActive,
      placement: "top-start",
      middleware: [
        offset(8),
        flip({ fallbackPlacements: ["bottom-start", "top-end", "bottom-end"] }),
        shift({ padding: 16 }),
        floatingSize({
          apply({ availableHeight, elements }) {
            elements.floating.style.maxHeight = `${Math.min(320, availableHeight - 16)}px`;
          },
        }),
      ],
      whileElementsMounted: autoUpdate,
    });

    useEffect(() => {
      if (containerRef.current) {
        refs.setReference(containerRef.current);
      }
    }, [refs]);

    useImperativeHandle(ref, () => ({
      focus: () => taRef.current?.focus(),
      clear: () => {
        setText("");
        setMentions([]);
        historyRef.current = [{ text: "", mentions: [], cursor: 0 }];
        historyIdxRef.current = 0;
        onContentChange?.(false);
        if (taRef.current) taRef.current.style.height = "auto";
      },
      getText: () => text,
      setText: (value: string) => {
        setText(value);
        setMentions([]);
        onContentChange?.(!!value.trim());
      },
      getMentions: () => mentions,
    }));

    const filteredItems = useMemo((): PopupItem[] => {
      if (triggerType === "/") {
        const cmds = slashCommands ?? [];
        return fuzzyFilter(cmds, popupQuery, (c) => c.label, (c) => c.desc)
          .map(({ item, result }) => ({ kind: "command" as const, command: item, result }));
      }
      return fuzzyFilter(options, popupQuery, (o) => o.label, (o) => o.desc)
        .map(({ item, result }) => ({ kind: "mention" as const, option: item, result }));
    }, [options, slashCommands, popupQuery, triggerType]);

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
        pushHistory(newVal, validMentions, cursorPos);
        onContentChange?.(!!newVal.trim());

        const beforeCursor = newVal.slice(0, cursorPos);

        const slashMatch = beforeCursor.match(/(?:^|\s)\/([^\s]*)$/);
        if (slashMatch) {
          setPopupActive(true);
          setTriggerType("/");
          setPopupQuery(slashMatch[1]);
          setPopupStart(cursorPos - slashMatch[0].length + (slashMatch[0].startsWith(" ") ? 1 : 0));
          setPopupIndex(0);
          return;
        }

        const atMatch = beforeCursor.match(/@([^\s@]*)$/);
        if (atMatch) {
          const isInsideMention = validMentions.some(
            (m) => cursorPos > m.start && cursorPos <= m.end,
          );
          if (!isInsideMention) {
            setPopupActive(true);
            setTriggerType("@");
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
      [text, updateMentionPositions, onContentChange, pushHistory],
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
            const shiftAmount = newText.length - text.length;
            if (m.start >= popupStart) {
              return { ...m, start: m.start + shiftAmount, end: m.end + shiftAmount };
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

    const handlePopupSelect = useCallback(
      (index: number) => {
        const item = filteredItems[index];
        if (!item) return;

        if (item.kind === "mention") {
          insertMention(item.option);
          return;
        }

        if (item.kind === "command") {
          const cursorPos = taRef.current?.selectionStart ?? text.length;
          const before = text.slice(0, popupStart);
          const after = text.slice(cursorPos);
          if (item.command.action) {
            setText(before + after);
            setPopupActive(false);
            setPopupQuery("");
            setPopupStart(-1);
            item.command.action();
          } else {
            const cmdText = `/${item.command.label} `;
            setText(before + cmdText + after);
            setPopupActive(false);
            setPopupQuery("");
            setPopupStart(-1);
            const newPos = popupStart + cmdText.length;
            setTimeout(() => {
              if (taRef.current) {
                taRef.current.setSelectionRange(newPos, newPos);
                taRef.current.focus();
              }
            }, 0);
          }
          return;
        }

      },
      [filteredItems, insertMention, text, popupStart],
    );

    const handleKeyDown = useCallback(
      (e: KeyboardEvent<HTMLTextAreaElement>) => {
        if (extraKeyHandler?.(e)) return;

        const isMod = e.metaKey || e.ctrlKey;

        if (isMod && e.key === "z" && !e.shiftKey) {
          e.preventDefault();
          undo();
          return;
        }
        if (isMod && (e.key === "y" || (e.key === "z" && e.shiftKey))) {
          e.preventDefault();
          redo();
          return;
        }

        if (popupActive) {
          if (e.key === "ArrowDown") {
            e.preventDefault();
            if (filteredItems.length > 0) {
              setPopupIndex((i) => (i + 1) % filteredItems.length);
            }
            return;
          }
          if (e.key === "ArrowUp") {
            e.preventDefault();
            if (filteredItems.length > 0) {
              setPopupIndex((i) => (i - 1 + filteredItems.length) % filteredItems.length);
            }
            return;
          }
          if (e.key === "Enter" || e.key === "Tab") {
            e.preventDefault();
            if (filteredItems.length > 0) {
              handlePopupSelect(popupIndex);
            }
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

        if (e.key === "ArrowUp" && !text.trim() && !popupActive && onRecallLastMessage) {
          const lastMsg = onRecallLastMessage();
          if (lastMsg) {
            e.preventDefault();
            setText(lastMsg);
            setMentions([]);
            setTimeout(() => {
              if (taRef.current) {
                taRef.current.setSelectionRange(lastMsg.length, lastMsg.length);
              }
            }, 0);
            return;
          }
        }
      },
      [
        text,
        mentions,
        popupActive,
        filteredItems,
        popupIndex,
        handlePopupSelect,
        insertMention,
        onSend,
        onNewTopic,
        onAttach,
        onRecallLastMessage,
        extraKeyHandler,
        undo,
        redo,
      ],
    );

    const MAX_HEIGHT = 240;

    const autoGrow = useCallback(() => {
      const el = taRef.current;
      if (!el) return;
      el.style.height = "auto";
      const clamped = Math.min(el.scrollHeight, MAX_HEIGHT);
      el.style.height = clamped + "px";
    }, []);

    const syncOverlayScroll = useCallback(() => {
      const ta = taRef.current;
      const overlay = ta?.parentElement?.querySelector<HTMLElement>(".mention-highlight-overlay");
      if (ta && overlay) overlay.scrollTop = ta.scrollTop;
    }, []);

    useEffect(() => {
      autoGrow();
    }, [text, autoGrow]);

    useEffect(() => {
      const el = taRef.current;
      if (!el) return;
      const ro = new ResizeObserver(() => autoGrow());
      ro.observe(el);
      return () => ro.disconnect();
    }, [autoGrow]);

    const popupEl = popupActive ? (
      <MentionPopup
        items={filteredItems}
        selectedIndex={popupIndex}
        onSelect={handlePopupSelect}
        triggerType={triggerType}
        query={popupQuery}
        floatingRef={refs.setFloating}
        floatingStyles={floatingStyles}
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
              e.preventDefault();

              // 同步读取 clipboardData（必须在 preventDefault 之后、异步之前读取）
              const clipText = e.clipboardData?.getData("text/plain") ?? "";
              const clipUri = e.clipboardData?.getData("text/uri-list") ?? "";
              const clipItems = e.clipboardData?.items;

              // 1) Web 标准：clipboardData 直接含图片文件（macOS/Windows 可用）
              if (clipItems && onPasteFiles) {
                const imageFiles: globalThis.File[] = [];
                for (const item of clipItems) {
                  if (item.type.startsWith("image/")) {
                    const file = item.getAsFile();
                    if (file) imageFiles.push(file);
                  }
                }
                if (imageFiles.length > 0) {
                  onPasteFiles(imageFiles);
                  return;
                }
              }

              const isTauri = "__TAURI_INTERNALS__" in window || "__TAURI__" in window;
              const textToCheck = (clipText || clipUri).trim();

              // 2) Tauri: file:// URL 或绝对路径指向图片
              if (isTauri && onPasteFiles) {
                const IMG_EXT = /\.(png|jpe?g|gif|webp|bmp|svg)$/i;
                const fileUrlMatch = textToCheck.match(/^file:\/\/(.+\.(png|jpe?g|gif|webp|bmp|svg))$/im);
                const isAbsImgPath = !fileUrlMatch && IMG_EXT.test(textToCheck) && textToCheck.startsWith("/");
                const filePathToRead = fileUrlMatch
                  ? decodeURIComponent(fileUrlMatch[1])
                  : isAbsImgPath ? textToCheck : null;

                if (filePathToRead) {
                  import("@tauri-apps/api/core").then(({ invoke }) => {
                    invoke<[string, string]>("read_image_file", { path: filePathToRead })
                      .then(([b64, mime]) => {
                        const arr = Uint8Array.from(atob(b64), (c) => c.charCodeAt(0));
                        const blob = new Blob([arr], { type: mime });
                        const name = filePathToRead.split("/").pop() ?? "image.png";
                        const file = new (globalThis.File as unknown as new (p: BlobPart[], n: string, o: FilePropertyBag) => globalThis.File)([blob], name, { type: mime });
                        onPasteFiles([file]);
                      })
                      .catch(() => {});
                  });
                  return;
                }
              }

              // 3) 有文本 → 手动插入到光标位置（不依赖浏览器默认粘贴行为）
              if (clipText) {
                const ta = taRef.current;
                const selStart = ta?.selectionStart ?? text.length;
                const selEnd = ta?.selectionEnd ?? selStart;
                const before = text.slice(0, selStart);
                const after = text.slice(selEnd);
                const newText = before + clipText + after;
                const newCursor = selStart + clipText.length;

                setText(newText);
                onContentChange?.(!!newText.trim());
                pushHistory(newText, mentions, newCursor);

                setTimeout(() => {
                  ta?.setSelectionRange(newCursor, newCursor);
                }, 0);
              }

              // 4) Tauri: 异步检查剪贴板是否有图片（截图场景）
              if (isTauri && onPasteFiles) {
                import("@tauri-apps/api/core").then(({ invoke }) => {
                  invoke<string | null>("clipboard_read_image")
                    .then((b64) => {
                      if (b64) {
                        const arr = Uint8Array.from(atob(b64), (c) => c.charCodeAt(0));
                        const blob = new Blob([arr], { type: "image/png" });
                        const file = new (globalThis.File as unknown as new (p: BlobPart[], n: string, o: FilePropertyBag) => globalThis.File)(
                          [blob], `screenshot-${Date.now()}.png`, { type: "image/png" }
                        );
                        onPasteFiles([file]);
                      }
                    })
                    .catch(() => {});
                });
              }
            }}
            placeholder={placeholder}
            rows={1}
            disabled={disabled}
            className="mention-textarea"
            spellCheck={false}
          />
        </div>
      </div>
    );
  },
);
