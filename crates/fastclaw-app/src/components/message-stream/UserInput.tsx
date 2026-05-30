import { useState, useMemo, useCallback, memo } from "react";
import { Copy, Check, Pencil } from "lucide-react";
import type { ChatMessage } from "../../lib/agent-store";
import { openLightbox } from "../common/ImageLightbox";

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

interface UserInputProps {
  msg: ChatMessage;
  copyable?: boolean;
  selected?: boolean;
  onToggleSelect?: () => void;
  animate?: boolean;
}

export const UserInput = memo(function UserInput({ msg, copyable, selected, onToggleSelect, animate = true }: UserInputProps) {
  const { text, tags } = useMemo(() => parseUserContent(msg.content), [msg.content]);
  const [copied, setCopied] = useState(false);
  const [hovered, setHovered] = useState(false);

  const handleCopy = useCallback(() => {
    navigator.clipboard.writeText(text).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    });
  }, [text]);

  const handleEdit = useCallback(() => {
    window.dispatchEvent(new CustomEvent("fastclaw:edit-message", { detail: { text, images: msg.images } }));
  }, [text, msg.images]);

  return (
    <div
      className="group/user-input mt-4 mb-3"
      style={{ animation: animate ? "fade-in var(--duration-fast) var(--ease-out)" : "none" }}
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
    >
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
        {/* User avatar */}
        <div
          className="w-[30px] h-[30px] rounded-full grid place-items-center text-[12px] font-bold text-white shrink-0"
          style={{ background: "linear-gradient(135deg, var(--tint), color-mix(in srgb, var(--tint) 70%, #6366F1))" }}
        >
          U
        </div>

        <div className="flex-1 min-w-0">
          {/* Header: label + timestamp + actions */}
          <div className="flex items-center gap-2 mb-1">
            <span className="text-[13px] font-semibold" style={{ color: "var(--fill-primary)" }}>
              You
            </span>
            <span className="shrink-0 text-[11px] tabular-nums" style={{ color: "var(--fill-quaternary)" }}>
              {ts(msg.timestamp)}
            </span>
            <span className="flex-1" />
            {/* Hover actions */}
            {copyable && hovered && (
              <div
                className="flex items-center gap-0.5"
                style={{ animation: "fade-in var(--duration-instant)" }}
              >
                <button
                  onClick={handleCopy}
                  className="flex h-5 w-5 items-center justify-center rounded transition-all duration-150 hover:bg-[var(--bg-hover)] active:scale-90 cursor-pointer"
                  style={{ color: copied ? "var(--green)" : "var(--fill-quaternary)" }}
                  title="复制"
                >
                  {copied ? <Check size={11} strokeWidth={1.5} /> : <Copy size={11} strokeWidth={1.2} />}
                </button>
                <button
                  onClick={handleEdit}
                  className="flex h-5 w-5 items-center justify-center rounded transition-all duration-150 hover:bg-[var(--bg-hover)] active:scale-90 cursor-pointer"
                  style={{ color: "var(--fill-quaternary)" }}
                  title="编辑"
                >
                  <Pencil size={11} strokeWidth={1.2} />
                </button>
              </div>
            )}
          </div>

          {/* Message bubble */}
          <div
            className="inline-block rounded-[14px_14px_14px_4px] px-4 py-3"
            style={{
              background: "var(--bg-surface)",
              border: "1px solid var(--separator)",
            }}
          >
            <div
              className="text-[14px] leading-[1.6] break-words"
              style={{ color: "var(--fill-primary)", overflowWrap: "anywhere" }}
            >
              {text}
            </div>

            {/* Images */}
            {msg.images && msg.images.length > 0 && (
              <div className="mt-2 flex flex-wrap gap-1.5">
                {msg.images.map((img, i) => (
                  <img
                    key={i}
                    src={img.url}
                    alt={img.alt || "attached image"}
                    className="cursor-pointer rounded-md object-cover"
                    style={{ maxHeight: 200, maxWidth: "100%", border: "0.5px solid var(--separator)" }}
                    loading="lazy"
                    onClick={() => openLightbox(img.url, img.alt || "attached image")}
                  />
                ))}
              </div>
            )}

            {/* Reference tags */}
            {tags.length > 0 && (
              <div className="mt-1.5 flex flex-wrap gap-1.5">
                {tags.map((tag, ti) =>
                  tag.items.map((item, ii) => (
                    <span
                      key={`${ti}-${ii}`}
                      className="inline-flex items-center gap-1 rounded-md px-2 py-0.5 text-[11px] font-medium"
                      style={{
                        background: "var(--bg-tertiary)",
                        color: "var(--fill-secondary)",
                        border: "0.5px solid var(--separator)",
                      }}
                    >
                      <span className="max-w-[120px] truncate">{tag.type}: {item}</span>
                    </span>
                  ))
                )}
              </div>
            )}
          </div>
        </div>
      </div>
    </div>
  );
});
