import { useState, useCallback } from "react";
import { useTranslation } from "react-i18next";
import { Clock, Timer, Calendar, CalendarBlank, GearSix } from "@phosphor-icons/react";

const PRESETS = [
  { id: "daily_9", labelKey: "presetDaily9", cron: "0 9 * * *", icon: Clock },
  { id: "hourly", labelKey: "presetHourly", cron: "0 * * * *", icon: Timer },
  { id: "every_4h", labelKey: "presetEvery4h", cron: "0 */4 * * *", icon: CalendarBlank },
  { id: "weekly_mon", labelKey: "presetWeeklyMon", cron: "0 9 * * 1", icon: Calendar },
  { id: "custom", labelKey: "presetCustom", cron: "", icon: GearSix },
] as const;

const DAY_KEYS = ["daySun", "dayMon", "dayTue", "dayWed", "dayThu", "dayFri", "daySat"] as const;

type CronTr = (key: string, opts?: Record<string, unknown>) => string;

export function cronToHuman(cron: string, t: CronTr): string {
  const parts = cron.trim().split(/\s+/);
  if (parts.length < 5) return cron;

  const [min, hour, dom, mon, dow] = parts;

  if (min === "0" && hour === "*" && dom === "*" && mon === "*" && dow === "*") return t("cronEveryHourAtZero");
  if (min.startsWith("*/")) return t("cronEveryMinutes", { count: min.slice(2) });
  if (min === "0" && hour.startsWith("*/")) return t("cronEveryHours", { count: hour.slice(2) });
  if (hour.startsWith("*/")) return t("cronEveryHoursAtMin", { count: hour.slice(2), min: min.padStart(2, "0") });

  const time = `${hour.padStart(2, "0")}:${min.padStart(2, "0")}`;

  if (dom === "*" && mon === "*" && dow === "*") return t("cronDailyAt", { time });

  if (dom === "*" && mon === "*" && dow !== "*") {
    const dayIdx = parseInt(dow, 10);
    const dayKey = DAY_KEYS[dayIdx];
    const dayName = dayKey ? t(dayKey) : dow;
    return t("cronWeeklyAt", { day: dayName, time });
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
  const { t } = useTranslation("automation");
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
              {t(p.labelKey)}
            </button>
          );
        })}
      </div>
      {mode === "custom" && (
        <input
          type="text"
          value={value}
          onChange={(e) => onChange(e.target.value)}
          placeholder={t("cronPlaceholder")}
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
          {isCustomInvalid ? t("invalidCron") : cronToHuman(value, t)}
        </span>
      </div>
    </div>
  );
}
