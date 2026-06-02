import {
  ChevronLeft, Bot, MessageSquare, Clock, Search,
  Wrench, Sparkles, ArrowRight,
} from "lucide-react";
import { ICON } from "../../lib/ui-tokens";

const FEATURES = [
  { icon: Bot, cssColor: "var(--tint)", title: "多 Agent 管理", desc: "创建和管理多个 AI Agent，各自独立配置模型、人设和工具" },
  { icon: Wrench, cssColor: "var(--orange, #ED8936)", title: "工具调用", desc: "Agent 可调用内置工具和 MCP 服务器扩展能力" },
  { icon: Clock, cssColor: "var(--purple, #B794F4)", title: "定时任务", desc: "通过 Cron 表达式设置周期任务，自动化日常工作" },
  { icon: Search, cssColor: "var(--green)", title: "联网搜索", desc: "Agent 可实时搜索互联网获取最新信息" },
  { icon: MessageSquare, cssColor: "var(--blue, #63B3ED)", title: "多轮对话", desc: "支持上下文感知的多轮对话，自动管理会话历史" },
  { icon: Sparkles, cssColor: "var(--yellow, #F6E05E)", title: "技能系统", desc: "通过技能扩展 Agent 的专业能力，支持自定义和社区共享" },
];

export function FeaturesStep({ onNext, onPrev }: { onNext: () => void; onPrev: () => void }) {
  return (
    <div className="relative">
      <div className="absolute -top-12 left-0 flex">
        <button
          onClick={onPrev}
          className="flex cursor-pointer items-center gap-1 text-[13px] font-medium transition-colors hover:opacity-80"
          style={{ color: "var(--fill-tertiary)" }}
        >
          <ChevronLeft {...ICON.md} />
          返回
        </button>
      </div>

      <div className="mb-6 text-center">
        <h2 className="text-[22px] font-bold" style={{ color: "var(--fill-primary)" }}>
          核心功能一览
        </h2>
        <p className="mt-2 text-[13px]" style={{ color: "var(--fill-tertiary)" }}>
          小林的主要能力
        </p>
      </div>

      <div className="grid grid-cols-2 gap-3">
        {FEATURES.map((f) => (
          <div
            key={f.title}
            className="rounded-[var(--radius-sm)] p-4 transition-all duration-200 hover:scale-[1.01]"
            style={{ background: "var(--bg-elevated)", border: "0.5px solid var(--separator-opaque)" }}
          >
            <div
              className="mb-3 flex h-9 w-9 items-center justify-center rounded-[8px]"
              style={{ background: `color-mix(in srgb, ${f.cssColor} 10%, transparent)` }}
            >
              <f.icon {...ICON.lg} style={{ color: f.cssColor }} />
            </div>
            <h3 className="text-[13px] font-semibold" style={{ color: "var(--fill-primary)" }}>
              {f.title}
            </h3>
            <p
              className="mt-1 text-[11px] leading-relaxed"
              style={{ color: "var(--fill-secondary)" }}
            >
              {f.desc}
            </p>
          </div>
        ))}
      </div>

      <div className="mt-6 flex justify-end">
        <button
          onClick={onNext}
          className="flex cursor-pointer items-center gap-2 rounded-full px-8 py-3 text-[14px] font-medium transition-all duration-200 hover:scale-[1.02] active:scale-[0.98]"
          style={{ background: "var(--fill-primary)", color: "var(--fill-inverse)" }}
        >
          开始使用
          <ArrowRight {...ICON.md} />
        </button>
      </div>
    </div>
  );
}
