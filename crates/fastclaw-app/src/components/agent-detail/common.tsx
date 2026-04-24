import { useState } from "react";
import { X, Search } from "lucide-react";

export const COLLAPSE_THRESHOLD = 10;

export function SectionHeader({ children, count, total, searchable, query, onQueryChange }: {
  children: React.ReactNode;
  count?: number;
  total?: number;
  searchable?: boolean;
  query?: string;
  onQueryChange?: (v: string) => void;
}) {
  const [showSearch, setShowSearch] = useState(false);
  return (
    <div className="mb-1.5 flex items-center gap-2">
      <label className="text-[11px] font-medium uppercase tracking-wider" style={{ color: "var(--fill-tertiary)" }}>
        {children}
      </label>
      {total != null && (
        <span className="text-[10px]" style={{ color: "var(--fill-quaternary)" }}>
          ({count ?? total}/{total})
        </span>
      )}
      <div className="flex-1" />
      {searchable && (
        showSearch ? (
          <div className="flex items-center gap-1">
            <input
              type="text"
              value={query ?? ""}
              onChange={(e) => onQueryChange?.(e.target.value)}
              placeholder="搜索..."
              className="w-28 bg-transparent text-[11px] outline-none"
              style={{ color: "var(--fill-primary)", borderBottom: "0.5px solid var(--separator)" }}
              autoFocus
            />
            <button onClick={() => { setShowSearch(false); onQueryChange?.(""); }} className="cursor-pointer" style={{ color: "var(--fill-quaternary)" }}>
              <X size={10} strokeWidth={2} />
            </button>
          </div>
        ) : (
          <button onClick={() => setShowSearch(true)} className="cursor-pointer transition-colors duration-100 hover:opacity-70" style={{ color: "var(--fill-quaternary)" }}>
            <Search size={11} strokeWidth={1.5} />
          </button>
        )
      )}
    </div>
  );
}

export function Toggle({ checked, onChange, disabled }: { checked: boolean; onChange: (v: boolean) => void; disabled?: boolean }) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      disabled={disabled}
      onClick={() => onChange(!checked)}
      className="relative inline-flex h-5 w-9 shrink-0 cursor-pointer items-center rounded-full transition-colors duration-200 disabled:cursor-not-allowed disabled:opacity-50"
      style={{ background: checked ? "var(--fill-tertiary)" : "var(--bg-tertiary)" }}
    >
      <span
        className="inline-block h-3.5 w-3.5 rounded-full shadow-sm transition-transform duration-200"
        style={{ background: "white", transform: checked ? "translateX(17px)" : "translateX(3px)" }}
      />
    </button>
  );
}

export function ListContainer({ children }: { children: React.ReactNode }) {
  return (
    <div className="overflow-hidden rounded-[var(--radius-sm)]" style={{ background: "var(--bg-elevated)", border: "0.5px solid var(--separator-opaque)" }}>
      {children}
    </div>
  );
}

export function EmptyRow({ text }: { text: string }) {
  return (
    <div className="px-3 py-3 text-center text-[12px]" style={{ color: "var(--fill-tertiary)" }}>
      {text}
    </div>
  );
}

export function FormModal({ open, onClose, title, children }: {
  open: boolean;
  onClose: () => void;
  title: string;
  children: React.ReactNode;
}) {
  if (!open) return null;
  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center"
      style={{ animation: "fade-in 0.15s ease-out" }}
      onKeyDown={(e) => { if (e.key === "Escape") onClose(); }}
      role="dialog"
      aria-modal="true"
      aria-label={title}
    >
      <div className="absolute inset-0" style={{ background: "rgba(0, 0, 0, 0.3)" }} onClick={onClose} role="presentation" />
      <div
        className="relative w-full max-w-[420px] overflow-hidden rounded-[var(--radius-md)]"
        style={{
          background: "var(--bg-elevated)",
          boxShadow: "var(--shadow-lg)",
          animation: "scale-in 0.2s ease-out",
          border: "0.5px solid var(--separator)",
          maxHeight: "calc(100vh - 80px)",
        }}
      >
        <div className="flex items-center justify-between px-5 py-3.5" style={{ borderBottom: "0.5px solid var(--separator)" }}>
          <h3 className="text-[14px] font-semibold" style={{ color: "var(--fill-primary)" }}>{title}</h3>
          <button onClick={onClose} className="flex h-6 w-6 cursor-pointer items-center justify-center rounded-full transition-colors duration-100 hover:bg-[var(--bg-hover)]" style={{ color: "var(--fill-tertiary)" }}>
            <X size={12} strokeWidth={2} />
          </button>
        </div>
        <div className="overflow-y-auto px-5 py-4" style={{ maxHeight: "calc(100vh - 160px)" }}>
          {children}
        </div>
      </div>
    </div>
  );
}

export function CollapsibleList<T>({ items, renderItem, emptyText }: {
  items: T[];
  renderItem: (item: T, index: number, isLast: boolean) => React.ReactNode;
  emptyText: string;
}) {
  const [showAll, setShowAll] = useState(false);
  const needsCollapse = items.length > COLLAPSE_THRESHOLD;
  const visible = needsCollapse && !showAll ? items.slice(0, COLLAPSE_THRESHOLD) : items;

  if (items.length === 0) return <ListContainer><EmptyRow text={emptyText} /></ListContainer>;

  return (
    <ListContainer>
      {visible.map((item, i) => renderItem(item, i, !needsCollapse ? i === items.length - 1 : showAll ? i === items.length - 1 : i === visible.length - 1 && !needsCollapse))}
      {needsCollapse && (
        <button
          onClick={() => setShowAll(!showAll)}
          className="w-full cursor-pointer px-3 py-2 text-center text-[11px] font-medium transition-colors duration-100 hover:bg-[var(--bg-hover)]"
          style={{ color: "var(--fill-tertiary)", borderTop: "0.5px solid var(--separator)" }}
        >
          {showAll ? "收起" : `显示全部 (${items.length})`}
        </button>
      )}
    </ListContainer>
  );
}
