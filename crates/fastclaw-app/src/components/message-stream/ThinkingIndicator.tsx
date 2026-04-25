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
        animation: "slide-left 0.2s ease-out",
        maxWidth: "75%",
      }}
    >
      {/* Animated dots */}
      <div className="flex items-center gap-1.5 h-5">
        {[0, 1, 2].map((i) => (
          <span
            key={i}
            style={{
              width: 6,
              height: 6,
              borderRadius: "50%",
              background: "var(--tint)",
              opacity: dots > i ? 1 : 0.25,
              transform: dots > i ? "scale(1)" : "scale(0.7)",
              transition: "opacity 0.3s ease, transform 0.3s ease",
            }}
          />
        ))}
      </div>

      {/* Label */}
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
