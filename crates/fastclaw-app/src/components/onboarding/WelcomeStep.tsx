import { Settings, ArrowRight } from "lucide-react";
import { ClawIcon } from "../layout/ClawIcon";

export function WelcomeStep({ onNext, onImport }: { onNext: () => void; onImport: () => void }) {
  return (
    <div className="flex flex-col items-center text-center">
      <div style={{ animation: "scale-in var(--duration-slower) var(--ease-out)" }}>
        <ClawIcon size={72} />
      </div>
      <h1
        className="mt-6 text-[28px] font-bold tracking-tight"
        style={{ color: "var(--fill-primary)" }}
      >
        欢迎使用 FastClaw
      </h1>
      <p
        className="mt-3 max-w-[380px] text-[15px] leading-relaxed"
        style={{ color: "var(--fill-secondary)" }}
      >
        FastClaw 是一个本地优先的 AI Agent 平台。
        <br />
        支持多 Agent 管理、工具调用、定时任务和联网搜索。
      </p>
      <p className="mt-6 text-[13px]" style={{ color: "var(--fill-tertiary)" }}>
        如何开始？
      </p>
      <div className="mt-4 flex w-full max-w-[280px] flex-col gap-3">
        <button
          onClick={onNext}
          className="flex cursor-pointer items-center justify-center gap-2 rounded-full px-6 py-3 text-[14px] font-medium transition-all duration-200 hover:scale-[1.02] active:scale-[0.98]"
          style={{ background: "var(--fill-primary)", color: "var(--fill-inverse)" }}
        >
          新手配置
          <Settings size={16} strokeWidth={2} />
        </button>
        <button
          onClick={onImport}
          className="flex cursor-pointer items-center justify-center gap-2 rounded-full px-6 py-3 text-[14px] font-medium transition-all duration-200 hover:scale-[1.02] active:scale-[0.98]"
          style={{
            background: "var(--bg-elevated)",
            color: "var(--fill-primary)",
            border: "1px solid var(--separator-opaque)",
          }}
        >
          导入现有配置
          <ArrowRight size={16} strokeWidth={2} />
        </button>
      </div>
    </div>
  );
}
