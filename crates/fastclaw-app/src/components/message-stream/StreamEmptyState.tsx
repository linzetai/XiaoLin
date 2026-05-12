import { useState, useMemo, useCallback } from "react";
import {
  FileText, Sparkles, Search, Settings2, Code2, MessageSquare,
  Palette, Globe, Lightbulb, PenTool, BarChart3, Shield,
  RefreshCw, Zap, BookOpen,
} from "lucide-react";
import { useAgentStore } from "../../lib/agent-store";

const SUGGESTION_POOL = [
  { title: "分析代码", desc: "解读和审查代码逻辑", icon: FileText, color: "var(--tint, #2563EB)" },
  { title: "API 设计", desc: "设计 RESTful 或 GraphQL 方案", icon: Sparkles, color: "var(--orange, #FF9500)" },
  { title: "排查 Bug", desc: "定位和修复代码问题", icon: Search, color: "var(--red, #FF3B30)" },
  { title: "性能优化", desc: "提升系统运行效率", icon: Zap, color: "var(--green, #34C759)" },
  { title: "写单元测试", desc: "为函数编写测试用例", icon: Shield, color: "#8B5CF6" },
  { title: "重构代码", desc: "改善代码结构和可读性", icon: Settings2, color: "#0EA5E9" },
  { title: "写文档", desc: "生成技术文档或 README", icon: BookOpen, color: "#D97706" },
  { title: "学习新技术", desc: "解释框架或库的用法", icon: Lightbulb, color: "#F59E0B" },
  { title: "UI 设计建议", desc: "提供界面设计灵感", icon: Palette, color: "#EC4899" },
  { title: "数据分析", desc: "分析数据并生成图表", icon: BarChart3, color: "#14B8A6" },
  { title: "翻译润色", desc: "翻译或优化文案表达", icon: Globe, color: "#6366F1" },
  { title: "头脑风暴", desc: "产品功能创意发散", icon: MessageSquare, color: "#F97316" },
  { title: "写脚本工具", desc: "自动化脚本或 CLI 工具", icon: Code2, color: "#10B981" },
  { title: "架构设计", desc: "系统架构方案讨论", icon: PenTool, color: "#8B5CF6" },
];

function shuffle<T>(arr: T[]): T[] {
  const copy = [...arr];
  for (let i = copy.length - 1; i > 0; i--) {
    const j = Math.floor(Math.random() * (i + 1));
    [copy[i], copy[j]] = [copy[j], copy[i]];
  }
  return copy;
}

export function StreamEmptyState({ onPick }: { onPick: (t: string) => void }) {
  const agents = useAgentStore((s) => s.agents);
  const activeAgentId = useAgentStore((s) => s.activeAgentId);
  const agent = agents.find((a) => a.id === activeAgentId) ?? agents[0];

  const [seed, setSeed] = useState(0);
  const cards = useMemo(() => shuffle(SUGGESTION_POOL).slice(0, 4), [seed]);

  const handleRefresh = useCallback(() => setSeed((s) => s + 1), []);

  return (
    <div
      className="flex h-full flex-col items-center justify-center px-8"
      style={{ animation: "scale-in var(--duration-slow) var(--ease-out)" }}
    >
      <div className="mb-8 text-center">
        <h2
          className="text-[26px] font-bold tracking-[-0.03em]"
          style={{ color: "var(--fill-primary)" }}
        >
          Hi，我是{agent?.name ?? "Agent"}
          <sup
            className="ml-0.5 text-[14px] font-semibold"
            style={{ color: "var(--tint)" }}
          >
            +
          </sup>
        </h2>
        {agent?.tagline && (
          <p className="mt-2 text-[14px]" style={{ color: "var(--fill-tertiary)" }}>
            {agent.tagline}
          </p>
        )}
      </div>

      <div className="w-full" style={{ maxWidth: 560 }}>
        <div className="grid grid-cols-2 gap-3 pb-4">
          {cards.map((card, i) => {
            const Icon = card.icon;
            return (
              <button
                key={`${card.title}-${seed}`}
                onClick={() => onPick(card.title)}
                className="group flex cursor-pointer flex-col gap-2.5 rounded-xl px-4 py-4 text-left transition-all duration-200 hover:-translate-y-0.5 active:translate-y-0 active:scale-[0.98]"
                style={{
                  background: "var(--bg-surface)",
                  border: "0.5px solid var(--border-subtle)",
                  boxShadow: "var(--shadow-md), inset 0 1px 0 var(--highlight-top)",
                  animation: `fade-slide-up var(--duration-slow) var(--ease-out) ${0.06 + i * 0.06}s backwards`,
                }}
              >
                <div
                  className="flex h-9 w-9 items-center justify-center rounded-lg transition-transform duration-200 group-hover:scale-110"
                  style={{ background: `color-mix(in srgb, ${card.color} 10%, transparent)`, color: card.color }}
                >
                  <Icon size={18} strokeWidth={1.5} />
                </div>
                <div>
                  <span
                    className="block text-[13px] font-semibold"
                    style={{ color: "var(--fill-primary)" }}
                  >
                    {card.title}
                  </span>
                  <span
                    className="mt-0.5 block text-[11px] leading-snug"
                    style={{ color: "var(--fill-quaternary)" }}
                  >
                    {card.desc}
                  </span>
                </div>
              </button>
            );
          })}
        </div>

        <div className="flex justify-end">
          <button
            onClick={handleRefresh}
            className="flex items-center gap-1.5 text-[12px] font-medium transition-colors duration-150 hover:opacity-70"
            style={{ color: "var(--fill-tertiary)" }}
          >
            <RefreshCw size={12} strokeWidth={1.5} />
            换一换
          </button>
        </div>
      </div>
    </div>
  );
}
