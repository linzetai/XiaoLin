import * as React from "react";
import { Check } from "lucide-react";
import { ACCENT_PRESETS } from "../../lib/theme";

export function SectionTitle({ children }: { children: React.ReactNode }) {
  return (
    <h3 className="mb-3 text-[11px] font-semibold uppercase tracking-wider" style={{ color: "var(--fill-tertiary)" }}>
      {children}
    </h3>
  );
}

export function SettingRow({ label, description, children, isLast }: { label: string; description?: string; children: React.ReactNode; isLast?: boolean }) {
  return (
    <div
      className="flex items-center justify-between px-4 py-3"
      style={!isLast ? { borderBottom: "0.5px solid var(--separator)" } : undefined}
    >
      <div className="mr-4 min-w-0 flex-1">
        <div className="text-[13px] font-medium" style={{ color: "var(--fill-primary)" }}>{label}</div>
        {description && (
          <div className="mt-0.5 text-[11px]" style={{ color: "var(--fill-tertiary)" }}>{description}</div>
        )}
      </div>
      {children}
    </div>
  );
}

export function Toggle({ enabled, onChange }: { enabled: boolean; onChange: () => void }) {
  return (
    <button
      onClick={onChange}
      className="relative inline-flex h-[22px] w-[40px] cursor-pointer shrink-0 items-center rounded-full"
      style={{
        background: enabled ? "var(--tint)" : "var(--separator-opaque)",
        transition: "background 0.2s var(--ease-in-out)",
      }}
    >
      <div
        className="h-[18px] w-[18px] rounded-full bg-white"
        style={{
          transform: enabled ? "translateX(20px)" : "translateX(2px)",
          boxShadow: "0 1px 3px rgba(0,0,0,0.15), 0 0 1px rgba(0,0,0,0.1)",
          transition: "transform 0.2s var(--ease-spring-subtle)",
        }}
      />
    </button>
  );
}

export function ThemeCard({ preset, selected, resolved, onClick }: {
  preset: typeof ACCENT_PRESETS[number];
  selected: boolean;
  resolved: "light" | "dark";
  onClick: () => void;
}) {
  const p = resolved === "dark" ? preset.preview.dark : preset.preview.light;

  return (
    <button
      onClick={onClick}
      className="group relative flex cursor-pointer flex-col items-center gap-2 outline-none focus-visible:outline-2 focus-visible:outline-offset-4"
      style={{ outlineColor: selected ? p.accent : "var(--tint)" } as React.CSSProperties}
      title={preset.label}
    >
      <div
        className="relative overflow-hidden rounded-[12px] transition-all duration-200 ease-out group-hover:scale-[1.04] group-active:scale-[0.98]"
        style={{
          width: 108,
          height: 72,
          background: p.bg,
          border: selected
            ? `2.5px solid ${p.accent}`
            : "1.5px solid var(--separator-opaque)",
          boxShadow: selected
            ? `0 0 0 2px ${p.accent}30, 0 4px 12px ${p.accent}25`
            : "0 1px 3px rgba(0,0,0,0.08)",
          transform: selected ? "scale(1.03)" : undefined,
        }}
      >
        <div
          className="absolute top-0 left-0 bottom-0"
          style={{ width: 26, background: p.sidebar, borderRight: `0.5px solid ${p.accent}18` }}
        />
        {[14, 24, 34].map((top) => (
          <div key={top} className="absolute left-[7px]" style={{ top, width: 12, height: 3, borderRadius: 1.5, background: p.text, opacity: 0.3 }} />
        ))}
        <div className="absolute left-[4px]" style={{ top: 8, width: 18, height: 3, borderRadius: 1.5, background: p.accent, opacity: 0.8 }} />
        <div className="absolute top-0 left-[26px] right-0" style={{ height: 14, background: p.sidebar, borderBottom: `0.5px solid ${p.accent}18` }} />
        <div className="absolute top-[5px] left-[32px]" style={{ width: 24, height: 3, borderRadius: 1.5, background: p.text, opacity: 0.5 }} />
        <div className="absolute top-[20px] left-[32px] right-[8px]" style={{ height: 3, borderRadius: 1.5, background: p.text, opacity: 0.2 }} />
        <div className="absolute top-[27px] left-[32px]" style={{ width: 38, height: 3, borderRadius: 1.5, background: p.text, opacity: 0.15 }} />
        <div className="absolute right-[6px]" style={{ top: 36, width: 34, height: 10, borderRadius: 5, background: p.accent, opacity: 0.9 }} />
        <div className="absolute left-[32px]" style={{ top: 50, width: 40, height: 10, borderRadius: 5, background: p.sidebar }} />
        <div className="absolute bottom-[4px] left-[30px] right-[4px]" style={{ height: 10, borderRadius: 4, background: p.sidebar, border: `0.5px solid ${p.accent}25` }} />
        {selected && (
          <div
            className="absolute top-[3px] right-[3px] flex items-center justify-center rounded-full"
            style={{
              width: 16, height: 16,
              background: p.accent,
              boxShadow: `0 1px 3px ${p.accent}60`,
              animation: "pop var(--duration-normal) var(--ease-spring)",
            }}
          >
            <Check size={12} strokeWidth={3} color="#fff" />
          </div>
        )}
      </div>
      <span
        className="text-[11px] font-medium transition-colors duration-150"
        style={{
          color: selected ? "var(--fill-primary)" : "var(--fill-tertiary)",
          fontWeight: selected ? 600 : 500,
        }}
      >
        {preset.label}
      </span>
    </button>
  );
}
