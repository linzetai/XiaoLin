import { useState, useEffect } from "react";
import { Sparkles, ArrowRight } from "lucide-react";
import { ICON } from "../../lib/ui-tokens";

export function DoneStep({ onComplete }: { onComplete: () => void }) {
  const [ready, setReady] = useState(false);

  useEffect(() => {
    const t = setTimeout(() => setReady(true), 1200);
    return () => clearTimeout(t);
  }, []);

  return (
    <div className="flex flex-col items-center text-center">
      <div
        className="flex h-16 w-16 items-center justify-center rounded-full"
        style={{
          background: "color-mix(in srgb, var(--green) 12%, transparent)",
          animation: "scale-in var(--duration-slow) var(--ease-out)",
        }}
      >
        <Sparkles size={32} strokeWidth={1.5} style={{ color: "var(--green)" }} />
      </div>
      <h2 className="mt-5 text-[22px] font-bold" style={{ color: "var(--fill-primary)" }}>
        一切就绪
      </h2>
      <p className="mt-2 text-[14px]" style={{ color: "var(--fill-secondary)" }}>
        准备好和你的 Agent 开始对话了
      </p>

      <div className="mt-6 flex gap-3">
        {ready && (
          <button
            onClick={onComplete}
            className="flex cursor-pointer items-center gap-2 rounded-full px-8 py-3 text-[14px] font-medium transition-all duration-200 hover:scale-[1.02] active:scale-[0.98]"
            style={{ background: "var(--fill-primary)", color: "var(--fill-inverse)" }}
          >
            开始使用
            <ArrowRight {...ICON.md} />
          </button>
        )}
      </div>
    </div>
  );
}
