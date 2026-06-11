import { Info, Sparkle } from "@phosphor-icons/react";
import type { BriefMessageData } from "../../lib/stores/types";

interface BriefMessageCardProps {
  data: BriefMessageData;
}

export function BriefMessageCard({ data }: BriefMessageCardProps) {
  const isProactive = data.mode === "proactive";

  return (
    <div
      className="mx-4 my-2 rounded-lg px-3.5 py-2.5 text-sm leading-relaxed"
      style={{
        borderLeft: `3px solid ${isProactive ? "var(--tint, #4299e1)" : "var(--fill-quaternary, #a0aec0)"}`,
        background: isProactive
          ? "var(--bg-tint-subtle, rgba(66, 153, 225, 0.06))"
          : "var(--bg-elevated, rgba(0, 0, 0, 0.03))",
        color: "var(--fill-secondary)",
      }}
    >
      <div className="flex items-start gap-2">
        {isProactive ? (
          <Sparkle className="mt-0.5 shrink-0" style={{ color: "var(--tint, #4299e1)" }} />
        ) : (
          <Info className="mt-0.5 shrink-0" style={{ color: "var(--fill-tertiary)" }} />
        )}
        <span className="min-w-0 whitespace-pre-wrap break-words">{data.content}</span>
      </div>
    </div>
  );
}
