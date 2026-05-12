import { useEffect, useState } from "react";

const LABELS = [
  "思考中",
  "正在思考",
  "处理中",
] as const;

export function ThinkingIndicator() {
  const [dots, setDots] = useState(0);
  const [labelIdx, setLabelIdx] = useState(0);

  useEffect(() => {
    const dotTimer = setInterval(() => setDots((d) => (d + 1) % 4), 500);
    const labelTimer = setInterval(
      () => setLabelIdx((i) => (i + 1) % LABELS.length),
      3000,
    );
    return () => {
      clearInterval(dotTimer);
      clearInterval(labelTimer);
    };
  }, []);

  return (
    <div
      className="pb-4 pl-2 flex items-center gap-2"
      style={{
        animation: "slide-left var(--duration-normal) var(--ease-out)",
        maxWidth: "75%",
      }}
    >
      <span
        className="text-[16px] leading-none"
        style={{
          color: "var(--tint)",
          animation: "sparkle-glow 2s ease-in-out infinite",
        }}
      >
        ✦
      </span>

      <span
        className="text-[13px]"
        style={{ color: "var(--fill-tertiary)" }}
      >
        {LABELS[labelIdx]}
        {".".repeat(dots)}
      </span>
    </div>
  );
}
