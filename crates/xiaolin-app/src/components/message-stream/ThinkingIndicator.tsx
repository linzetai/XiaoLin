import { useEffect, useState } from "react";

const LABELS = [
  "思考中",
  "正在思考",
  "处理中",
] as const;

function OrbitSpinner() {
  return (
    <svg
      width={20}
      height={20}
      viewBox="0 0 20 20"
      fill="none"
      className="shrink-0"
      style={{ animation: "spin 3s linear infinite" }}
    >
      <circle
        cx={10} cy={10} r={7.5}
        stroke="var(--tint)"
        strokeWidth={2}
        strokeDasharray="14 33"
        strokeLinecap="round"
        opacity={0.85}
        style={{ animation: "orbit 1.5s ease-in-out infinite" }}
      />
      <circle
        cx={10} cy={10} r={7.5}
        stroke="var(--tint)"
        strokeWidth={2}
        strokeDasharray="8 39"
        strokeLinecap="round"
        opacity={0.45}
        style={{ animation: "orbit 2.2s ease-in-out infinite reverse", transformOrigin: "center" }}
      />
      <circle
        cx={10} cy={10} r={7.5}
        stroke="var(--tint)"
        strokeWidth={1.5}
        strokeDasharray="5 42"
        strokeLinecap="round"
        opacity={0.3}
        style={{ animation: "orbit 1.8s ease-in-out infinite", transformOrigin: "center" }}
      />
    </svg>
  );
}

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
      className="pb-4 pl-2 flex items-center gap-2.5"
      style={{
        animation: "slide-left var(--duration-normal) var(--ease-out)",
        maxWidth: "75%",
      }}
    >
      <OrbitSpinner />
      <span
        className="text-[13px]"
        style={{
          color: "var(--fill-tertiary)",
          animation: "glow-pulse 2s ease-in-out infinite",
        }}
      >
        {LABELS[labelIdx]}
        {".".repeat(dots)}
      </span>
    </div>
  );
}
