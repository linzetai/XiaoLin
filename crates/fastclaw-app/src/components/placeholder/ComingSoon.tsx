import { Sparkles } from "lucide-react";

export function ComingSoon({ title }: { title?: string }) {
  return (
    <div
      className="flex h-full flex-col items-center justify-center gap-3"
      style={{ background: "var(--bg-primary)", animation: "scale-in var(--duration-slow) var(--ease-out)" }}
    >
      <div
        className="flex h-14 w-14 items-center justify-center rounded-2xl"
        style={{ background: "var(--tint-bg)", color: "var(--tint)" }}
      >
        <Sparkles size={24} strokeWidth={1.5} />
      </div>
      {title && (
        <h3
          className="text-[15px] font-semibold tracking-[-0.01em]"
          style={{ color: "var(--fill-primary)" }}
        >
          {title}
        </h3>
      )}
      <p className="text-[13px]" style={{ color: "var(--fill-tertiary)" }}>
        功能正在路上了，敬请期待
      </p>
    </div>
  );
}
