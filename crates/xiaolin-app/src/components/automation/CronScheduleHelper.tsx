import { useState, useCallback } from "react";
import { Clock, Timer, Calendar, CalendarBlank, GearSix } from "@phosphor-icons/react";

const PRESETS = [
  { id: "daily_9", label: "Daily 9:00", cron: "0 9 * * *", icon: Clock },
  { id: "hourly", label: "Every hour", cron: "0 * * * *", icon: Timer },
  { id: "every_4h", label: "Every 4 hours", cron: "0 */4 * * *", icon: CalendarBlank },
  { id: "weekly_mon", label: "Weekly Mon", cron: "0 9 * * 1", icon: Calendar },
  { id: "custom", label: "Custom", cron: "", icon: GearSix },
] as const;

export function cronToHuman(cron: string): string {
  const parts = cron.trim().split(/\s+/);
  if (parts.length < 5) return cron;

  const [min, hour, dom, mon, dow] = parts;

  if (min === "0" && hour === "*" && dom === "*" && mon === "*" && dow === "*") return "Every hour at :00";
  if (min.startsWith("*/")) return `Every ${min.slice(2)} minutes`;
  if (min === "0" && hour.startsWith("*/")) return `Every ${hour.slice(2)} hours`;
  if (hour.startsWith("*/")) return `Every ${hour.slice(2)} hours at :${min.padStart(2, "0")}`;

  const time = `${hour.padStart(2, "0")}:${min.padStart(2, "0")}`;

  if (dom === "*" && mon === "*" && dow === "*") return `Daily at ${time}`;

  const DAYS = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
  if (dom === "*" && mon === "*" && dow !== "*") {
    const dayIdx = parseInt(dow, 10);
    const dayName = DAYS[dayIdx] ?? dow;
    return `Weekly ${dayName} at ${time}`;
  }

  return cron;
}

function isValidCron(cron: string): boolean {
  const parts = cron.trim().split(/\s+/);
  if (parts.length !== 5) return false;
  return parts.every((p) => /^(\*|(\*\/\d+)|(\d+(-\d+)?(,\d+(-\d+)?)*))$/.test(p));
}

interface CronScheduleHelperProps {
  value: string;
  onChange: (cron: string) => void;
}

export function CronScheduleHelper({ value, onChange }: CronScheduleHelperProps) {
  const matchedPreset = PRESETS.find((p) => p.cron === value);
  const [mode, setMode] = useState<string>(matchedPreset?.id ?? "custom");

  const handlePreset = useCallback(
    (presetId: string) => {
      setMode(presetId);
      const preset = PRESETS.find((p) => p.id === presetId);
      if (preset && preset.cron) {
        onChange(preset.cron);
      }
    },
    [onChange],
  );

  const isCustomInvalid = mode === "custom" && value.trim() !== "" && !isValidCron(value);

  return (
    <div className="flex flex-col gap-2.5">
      <div className="flex flex-wrap gap-1.5">
        {PRESETS.map((p) => {
          const active = mode === p.id;
          const Icon = p.icon;
          return (
            <button
              key={p.id}
              type="button"
              onClick={() => handlePreset(p.id)}
              className="auto-btn flex items-center gap-1.5 rounded-[var(--radius-xs)] px-2.5 py-1.5 text-[11px] font-medium transition-all duration-150"
              style={{
                background: active ? "color-mix(in srgb, var(--tint) 12%, transparent)" : "var(--bg-primary)",
                color: active ? "var(--tint)" : "var(--fill-secondary)",
                border: `1px solid ${active ? "var(--tint)" : "var(--separator)"}`,
              }}
              aria-pressed={active}
            >
              <Icon size={11} weight="regular" />
              {p.label}
            </button>
          );
        })}
      </div>
      {mode === "custom" && (
        <input
          type="text"
          value={value}
          onChange={(e) => onChange(e.target.value)}
          placeholder="* * * * * (min hour dom mon dow)"
          className="auto-input px-3 py-2 text-[13px] font-mono outline-none transition-all duration-150 rounded-[var(--radius-xs)]"
          style={{
            background: "var(--bg-primary)",
            border: `1px solid ${isCustomInvalid ? "var(--red, #E53E3E)" : "var(--separator)"}`,
            color: "var(--fill-primary)",
          }}
          aria-invalid={isCustomInvalid}
        />
      )}
      <div className="flex items-center gap-2 text-[10px]">
        <Clock size={10} style={{ color: "var(--fill-quaternary)" }} />
        <span style={{ color: isCustomInvalid ? "var(--red, #E53E3E)" : "var(--fill-quaternary)" }}>
          {isCustomInvalid ? "Invalid cron expression" : cronToHuman(value)}
        </span>
      </div>
    </div>
  );
}
