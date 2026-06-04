import { Lock, Shield, ShieldCheck, ShieldOff, ChevronDown } from "lucide-react";
import { useState, useEffect, useRef, useCallback } from "react";
import { createPortal } from "react-dom";
import { usePermissionStore } from "../../lib/stores/permission-store";

const PRESET_ICONS: Record<string, typeof Shield> = {
  suggest: Shield,
  "auto-edit": ShieldCheck,
  "full-auto": ShieldOff,
  "plan-only": Lock,
};

const PRESET_COLORS: Record<string, string> = {
  suggest: "var(--fill-tertiary)",
  "auto-edit": "var(--tint, #4299E1)",
  "full-auto": "var(--orange, #ED8936)",
  "plan-only": "oklch(56% 0.18 310)",
};

interface PermissionSelectorProps {
  sessionId: string | undefined;
  disabled?: boolean;
}

export function PermissionSelector({ sessionId, disabled }: PermissionSelectorProps) {
  const presets = usePermissionStore((s) => s.presets);
  const presetsLoaded = usePermissionStore((s) => s.presetsLoaded);
  const loadPresets = usePermissionStore((s) => s.loadPresets);
  const getSessionPreset = usePermissionStore((s) => s.getSessionPreset);
  const setSessionPreset = usePermissionStore((s) => s.setSessionPreset);
  const fetchSessionPreset = usePermissionStore((s) => s.fetchSessionPreset);
  const [open, setOpen] = useState(false);
  const btnRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (!presetsLoaded) loadPresets();
  }, [presetsLoaded, loadPresets]);

  useEffect(() => {
    if (sessionId) fetchSessionPreset(sessionId);
  }, [sessionId, fetchSessionPreset]);

  const activePresetId = sessionId ? getSessionPreset(sessionId) : "";
  const activePreset = presets.find((p) => p.id === activePresetId);
  const displayName = activePreset?.name ?? "Suggest edits";
  const IconComponent = PRESET_ICONS[activePresetId] ?? Shield;
  const iconColor = PRESET_COLORS[activePresetId] ?? "var(--fill-tertiary)";

  const handleSelect = useCallback(
    async (presetId: string) => {
      setOpen(false);
      if (!sessionId) return;
      await setSessionPreset(sessionId, presetId);
    },
    [sessionId, setSessionPreset],
  );

  return (
    <div className="relative">
      <button
        ref={btnRef}
        onClick={() => setOpen(!open)}
        disabled={disabled}
        className="flex items-center gap-1 rounded-[5px] border-none bg-transparent px-[7px] py-[3px] text-[11px] font-medium whitespace-nowrap transition-colors duration-100 hover:bg-[var(--bg-hover)] disabled:cursor-not-allowed disabled:opacity-50"
        style={{ color: iconColor, cursor: disabled ? "not-allowed" : "pointer" }}
      >
        <IconComponent size={13} strokeWidth={1.6} />
        <span>{displayName}</span>
        <ChevronDown size={10} strokeWidth={1.5} style={{ opacity: 0.5, marginLeft: 1 }} />
      </button>
      {open &&
        createPortal(
          <div className="fixed inset-0 z-[60]" onClick={() => setOpen(false)}>
            <div
              className="fixed overflow-hidden rounded-xl py-1.5"
              style={{
                left: btnRef.current?.getBoundingClientRect().left ?? 0,
                bottom:
                  window.innerHeight -
                  (btnRef.current?.getBoundingClientRect().top ?? 0) +
                  4,
                minWidth: 220,
                background: "var(--bg-elevated)",
                border: "0.5px solid var(--separator)",
                boxShadow: "var(--shadow-lg)",
                animation: "scale-in var(--duration-fast) var(--ease-out)",
                transformOrigin: "bottom left",
              }}
              onClick={(e) => e.stopPropagation()}
            >
              <div
                className="px-3 pb-1.5 pt-1 text-[10px] font-semibold uppercase tracking-wider"
                style={{ color: "var(--fill-quaternary)" }}
              >
                权限预设
              </div>
              {presets.map((p) => {
                const active = p.id === activePresetId;
                const Icon = PRESET_ICONS[p.id] ?? Shield;
                const color = PRESET_COLORS[p.id] ?? "var(--fill-tertiary)";
                return (
                  <button
                    key={p.id}
                    onClick={() => handleSelect(p.id)}
                    className="flex w-full items-start gap-2.5 px-3 py-2 text-left transition-colors duration-100 hover:bg-[var(--bg-hover)]"
                    style={{
                      background: active
                        ? "color-mix(in srgb, var(--tint) 6%, transparent)"
                        : undefined,
                    }}
                  >
                    <Icon
                      size={14}
                      strokeWidth={1.6}
                      className="mt-0.5 shrink-0"
                      style={{ color }}
                    />
                    <div className="min-w-0">
                      <div
                        className="text-[12px] font-medium"
                        style={{
                          color: active
                            ? "var(--tint)"
                            : "var(--fill-secondary)",
                        }}
                      >
                        {p.name}
                      </div>
                      <div
                        className="mt-0.5 text-[10px]"
                        style={{ color: "var(--fill-quaternary)" }}
                      >
                        {p.description}
                      </div>
                    </div>
                    {active && (
                      <span
                        className="ml-auto mt-0.5 h-2 w-2 shrink-0 rounded-full"
                        style={{ background: "var(--tint)" }}
                      />
                    )}
                  </button>
                );
              })}
            </div>
          </div>,
          document.body,
        )}
    </div>
  );
}
