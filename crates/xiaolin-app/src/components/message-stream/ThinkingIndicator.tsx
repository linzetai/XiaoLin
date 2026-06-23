import { useEffect, useState, useMemo, useRef } from "react";
import { useTranslation } from "react-i18next";

export type Phase = "connecting" | "thinking" | "planning";

interface PhaseIndicatorProps {
  phase?: Phase;
}

export function PhaseIndicator({ phase = "thinking" }: PhaseIndicatorProps) {
  const { t } = useTranslation("chat");
  const [elapsed, setElapsed] = useState(0);
  const startRef = useRef(Date.now());

  const label = useMemo(() => {
    switch (phase) {
      case "connecting": return t("thinking_connecting");
      case "planning": return t("thinking_planning");
      case "thinking":
      default: return t("thinking_0");
    }
  }, [phase, t]);

  useEffect(() => {
    startRef.current = Date.now();
    setElapsed(0);
  }, [phase]);

  useEffect(() => {
    const timer = setInterval(() => setElapsed(Math.floor((Date.now() - startRef.current) / 1000)), 200);
    return () => clearInterval(timer);
  }, [phase]);

  return (
    <div
      className="pb-3 pl-3 flex items-center gap-2"
      style={{
        animation: "slide-left var(--duration-normal) var(--ease-out)",
        maxWidth: "min(90%, var(--content-max-w, 720px))",
      }}
    >
      {/* Pulsing dot */}
      <span
        className="inline-block h-[8px] w-[8px] rounded-full shrink-0"
        style={{
          background: "var(--tint)",
          animation: "phase-pulse 1.5s ease-in-out infinite",
        }}
      />
      <span
        className="text-[12px]"
        style={{ color: "var(--fill-tertiary)" }}
      >
        {label}
      </span>
      {elapsed > 0 && (
        <span
          className="text-[11px] tabular-nums"
          style={{ color: "var(--fill-quaternary)" }}
        >
          {elapsed}s
        </span>
      )}
    </div>
  );
}

export function ThinkingIndicator() {
  return <PhaseIndicator phase="thinking" />;
}
