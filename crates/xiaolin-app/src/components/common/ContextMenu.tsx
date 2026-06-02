import { useState, useEffect, useCallback, useRef, type ReactNode } from "react";
import { Copy, ClipboardPaste, Scissors, TextSelect } from "lucide-react";

interface MenuItem {
  label: string;
  icon?: ReactNode;
  shortcut?: string;
  action: () => void;
  disabled?: boolean;
  separator?: false;
}

interface MenuSeparator {
  separator: true;
}

type MenuEntry = MenuItem | MenuSeparator;

const ICON_PROPS = { size: 14, strokeWidth: 1.2 } as const;

function isSeparator(entry: MenuEntry): entry is MenuSeparator {
  return "separator" in entry && entry.separator === true;
}

function buildMenuItems(target: HTMLElement): MenuEntry[] {
  const sel = window.getSelection();
  const hasSelection = sel !== null && sel.toString().trim().length > 0;

  const isEditable =
    target instanceof HTMLInputElement ||
    target instanceof HTMLTextAreaElement ||
    target.isContentEditable;

  const items: MenuEntry[] = [];

  if (hasSelection) {
    items.push({
      label: "剪切",
      icon: <Scissors {...ICON_PROPS} />,
      shortcut: "Ctrl+X",
      disabled: !isEditable,
      action: () => document.execCommand("cut"),
    });
    items.push({
      label: "复制",
      icon: <Copy {...ICON_PROPS} />,
      shortcut: "Ctrl+C",
      action: () => navigator.clipboard.writeText(sel!.toString()),
    });
  }

  if (isEditable) {
    items.push({
      label: "粘贴",
      icon: <ClipboardPaste {...ICON_PROPS} />,
      shortcut: "Ctrl+V",
      action: async () => {
        try {
          const { invoke } = await import("@tauri-apps/api/core");
          const b64 = await invoke<string | null>("clipboard_read_image");
          if (b64) {
            const bin = atob(b64);
            const arr = new Uint8Array(bin.length);
            for (let i = 0; i < bin.length; i++) arr[i] = bin.charCodeAt(i);
            const blob = new Blob([arr], { type: "image/png" });
            const file = new (File as unknown as new (p: BlobPart[], n: string, o: FilePropertyBag) => File)(
              [blob], `pasted-${Date.now()}.png`, { type: "image/png" }
            );
            window.dispatchEvent(new CustomEvent("xiaolin:paste-files", { detail: { files: [file] } }));
          } else {
            const text = await navigator.clipboard.readText();
            document.execCommand("insertText", false, text);
          }
        } catch {
          try {
            const text = await navigator.clipboard.readText();
            document.execCommand("insertText", false, text);
          } catch { /* empty */ }
        }
      },
    });
  }

  if (items.length > 0) items.push({ separator: true });

  items.push({
    label: "全选",
    icon: <TextSelect {...ICON_PROPS} />,
    shortcut: "Ctrl+A",
    action: () => {
      if (isEditable && target instanceof HTMLInputElement) {
        target.select();
      } else if (isEditable && target instanceof HTMLTextAreaElement) {
        target.select();
      } else {
        document.execCommand("selectAll");
      }
    },
  });

  return items;
}

export function ContextMenuProvider() {
  const [pos, setPos] = useState<{ x: number; y: number } | null>(null);
  const [items, setItems] = useState<MenuEntry[]>([]);
  const menuRef = useRef<HTMLDivElement>(null);

  const close = useCallback(() => setPos(null), []);

  useEffect(() => {
    const onCtx = (e: MouseEvent) => {
      e.preventDefault();
      const target = e.target as HTMLElement;
      const builtItems = buildMenuItems(target);
      setItems(builtItems);

      const x = Math.min(e.clientX, window.innerWidth - 200);
      const y = Math.min(e.clientY, window.innerHeight - builtItems.length * 32 - 16);
      setPos({ x, y });
    };

    const onClick = (e: MouseEvent) => {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
        close();
      }
    };

    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") close();
    };

    window.addEventListener("contextmenu", onCtx);
    window.addEventListener("mousedown", onClick);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("contextmenu", onCtx);
      window.removeEventListener("mousedown", onClick);
      window.removeEventListener("keydown", onKey);
    };
  }, [close]);

  if (!pos) return null;

  return (
    <div
      ref={menuRef}
      className="fixed z-[9999] min-w-[160px] overflow-hidden rounded-lg py-1"
      style={{
        left: pos.x,
        top: pos.y,
        background: "var(--bg-elevated)",
        border: "0.5px solid var(--separator)",
        boxShadow: "0 8px 30px rgba(0,0,0,0.18), 0 1px 4px rgba(0,0,0,0.1)",
        backdropFilter: "blur(20px)",
        WebkitBackdropFilter: "blur(20px)",
        animation: "scale-spring 120ms var(--ease-out)",
      }}
    >
      {items.map((entry, i) => {
        if (isSeparator(entry)) {
          return (
            <div
              key={`sep-${i}`}
              className="mx-2 my-1 h-px"
              style={{ background: "var(--separator)" }}
            />
          );
        }
        return (
          <button
            key={entry.label}
            disabled={entry.disabled}
            onClick={() => {
              entry.action();
              close();
            }}
            className="flex w-full items-center gap-2.5 px-3 py-1.5 text-left text-[13px] transition-colors duration-75 hover:bg-[var(--bg-hover)] disabled:opacity-40 disabled:cursor-default"
            style={{ color: "var(--fill-primary)" }}
          >
            <span style={{ color: "var(--fill-tertiary)" }}>{entry.icon}</span>
            <span className="flex-1">{entry.label}</span>
            {entry.shortcut && (
              <span className="ml-4 text-[11px]" style={{ color: "var(--fill-quaternary)" }}>
                {entry.shortcut}
              </span>
            )}
          </button>
        );
      })}
    </div>
  );
}
