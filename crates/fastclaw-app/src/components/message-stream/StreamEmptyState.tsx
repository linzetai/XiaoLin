import { FileText, Sparkles, Search, Settings2 } from "lucide-react";
import { ClawIcon } from "../layout/ClawIcon";

export function StreamEmptyState({ onPick }: { onPick: (t: string) => void }) {
  const suggestions = [
    { text: "帮我分析一段代码", icon: <FileText size={15} strokeWidth={1.5} />, color: "var(--tint, #2563EB)" },
    { text: "写一个 API 设计方案", icon: <Sparkles size={15} strokeWidth={1.5} />, color: "var(--orange, #FF9500)" },
    { text: "排查一个 Bug", icon: <Search size={15} strokeWidth={1.5} />, color: "var(--red, #FF3B30)" },
    { text: "优化系统性能", icon: <Settings2 size={15} strokeWidth={1.5} />, color: "var(--green, #34C759)" },
  ];

  return (
    <div className="flex h-full flex-col items-center justify-center px-8" style={{ animation: "scale-in 0.35s ease-out" }}>
      <div className="mb-6" style={{ animation: "scale-in 0.5s ease-out" }}>
        <ClawIcon size={56} />
      </div>
      <h2 className="mb-2 text-[18px] font-semibold tracking-[-0.02em]" style={{ color: "var(--fill-primary)" }}>
        开始新的对话
      </h2>
      <p className="mb-8 text-[13px]" style={{ color: "var(--fill-tertiary)" }}>
        描述你的任务，或选择一个话题快速开始
      </p>
      <div className="grid grid-cols-2 gap-3" style={{ maxWidth: 400 }}>
        {suggestions.map((s, i) => (
          <button
            key={s.text}
            onClick={() => onPick(s.text)}
            className="group flex cursor-pointer items-center gap-3 rounded-[var(--radius-sm)] px-4 py-3.5 text-left text-[13px] transition-all duration-200 hover:shadow-[var(--shadow-sm)]"
            style={{
              background: "var(--bg-secondary)",
              border: "0.5px solid var(--separator)",
              color: "var(--fill-secondary)",
              animation: `slide-up 0.3s ease-out ${0.06 + i * 0.06}s backwards`,
            }}
          >
            <span
              className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg transition-transform duration-200 group-hover:scale-110"
              style={{ background: "var(--tint-subtle)", color: s.color }}
            >
              {s.icon}
            </span>
            <span className="min-w-0 truncate font-medium">{s.text}</span>
          </button>
        ))}
      </div>
    </div>
  );
}
