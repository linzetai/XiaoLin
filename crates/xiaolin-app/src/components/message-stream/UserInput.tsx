import { useState, useMemo, useCallback, memo } from "react";
import { useTranslation } from "react-i18next";
import { Copy, Check, PencilSimple } from "@phosphor-icons/react";
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


interface UserInputProps {
  msg: ChatMessage;
  copyable?: boolean;
  selected?: boolean;
  onToggleSelect?: () => void;
}

export const UserInput = memo(function UserInput({ msg, copyable, selected, onToggleSelect }: UserInputProps) {
  const { t } = useTranslation("chat");
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
    window.dispatchEvent(new CustomEvent("xiaolin:edit-message", { detail: { text, images: msg.images } }));
  }, [text, msg.images]);

  return (
    <div
      className="group/user-input mb-4"
      style={{ display: "flex", justifyContent: "flex-end" }}
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
    >
      {onToggleSelect && (
        <button
          onClick={onToggleSelect}
          className="mr-2 mt-2 flex h-4 w-4 shrink-0 items-center justify-center rounded border transition-colors duration-100 hover:border-[var(--fill-secondary)]"
          style={{
            borderColor: selected ? "var(--tint)" : "var(--fill-quaternary)",
            background: selected ? "var(--tint)" : "transparent",
          }}
        >
          {selected && <Check size={14} weight="bold" style={{ color: "white" }} />}
        </button>
      )}
      <div
        className="relative"
        style={{
          background: "var(--bg-user-msg)",
          borderRadius: 14,
          padding: "10px 16px",
          maxWidth: "70%",
          width: "fit-content",
        }}
      >
        {msg.isSteer && (
          <span
            className="absolute -top-2 right-2 rounded-full px-1.5 py-0.5 text-[10px] font-medium"
            style={{ background: "var(--tint-subtle, rgba(66,153,225,0.12))", color: "var(--tint)" }}
          >
            {t("steerAppend")}
          </span>
        )}
        <div className="text-[14px] leading-[1.5] break-words" style={{ color: "var(--fill-primary)", overflowWrap: "anywhere" }}>
          {text}
        </div>

        {msg.images && msg.images.length > 0 && (
          <div className="mt-2 flex flex-wrap gap-1.5">
            {msg.images.map((img, i) => (
              <img
                key={i} src={img.url} alt={img.alt || "attached image"}
                className="cursor-pointer rounded-md object-cover"
                style={{ maxHeight: 200, maxWidth: "100%", border: "0.5px solid var(--separator)" }}
                loading="lazy"
                onClick={() => openLightbox(img.url, img.alt || "attached image")}
              />
            ))}
          </div>
        )}

        {tags.length > 0 && (
          <div className="mt-1.5 flex flex-wrap gap-1.5">
            {tags.map((tag, ti) =>
              tag.items.map((item, ii) => (
                <span
                  key={`${ti}-${ii}`}
                  className="inline-flex items-center gap-1 rounded-md px-2 py-0.5 text-[11px] font-medium"
                  style={{ background: "color-mix(in srgb, var(--fill-primary) 6%, transparent)", color: "var(--fill-secondary)" }}
                >
                  <span className="max-w-[120px] truncate">
                    {tag.type === "引用" ? t("refTag") : tag.type === "附件" ? t("attachTag") : tag.type}: {item}
                  </span>
                </span>
              ))
            )}
          </div>
        )}

        {/* Hover actions */}
        {copyable && hovered && (
          <div
            className="absolute -bottom-3 right-2 flex items-center gap-0.5 rounded-md px-1 py-0.5"
            style={{ background: "var(--bg-elevated)", boxShadow: "var(--shadow-sm)", border: "0.5px solid var(--border-subtle)" }}
          >
            <button onClick={handleCopy}
              className="flex h-5 w-5 cursor-pointer items-center justify-center rounded transition-all duration-150 hover:bg-[var(--bg-hover)] active:scale-90"
              style={{ color: copied ? "var(--green)" : "var(--fill-quaternary)" }} title={t("copy", { ns: "common" })}
            >
              {copied ? <Check size={11} weight="fill" /> : <Copy size={11} weight="light" />}
            </button>
            <button onClick={handleEdit}
              className="flex h-5 w-5 cursor-pointer items-center justify-center rounded transition-all duration-150 hover:bg-[var(--bg-hover)] active:scale-90"
              style={{ color: "var(--fill-quaternary)" }} title={t("edit")}
            >
              <PencilSimple size={11} weight="light" />
            </button>
          </div>
        )}
      </div>
    </div>
  );
});
