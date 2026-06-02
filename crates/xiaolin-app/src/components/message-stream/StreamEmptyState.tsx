import { useState, useMemo, useCallback, useRef } from "react";
import {
  FileText, Sparkles, Search, Settings2, Code2, MessageSquare,
  Palette, Globe, Lightbulb, PenTool, BarChart3, Shield,
  RefreshCw, Zap, BookOpen,
} from "lucide-react";
import { useAgentStore } from "../../lib/agent-store";

import { ICON } from "../../lib/ui-tokens";

const SUGGESTION_POOL = [
  { title: "分析代码", desc: "解读和审查代码逻辑", icon: FileText, color: "var(--tint)" },
  { title: "API 设计", desc: "设计 RESTful 或 GraphQL 方案", icon: Sparkles, color: "var(--orange)" },
  { title: "排查 Bug", desc: "定位和修复代码问题", icon: Search, color: "var(--red)" },
  { title: "性能优化", desc: "提升系统运行效率", icon: Zap, color: "var(--green)" },
  { title: "写单元测试", desc: "为函数编写测试用例", icon: Shield, color: "var(--tint)" },
  { title: "重构代码", desc: "改善代码结构和可读性", icon: Settings2, color: "var(--tint)" },
  { title: "写文档", desc: "生成技术文档或 README", icon: BookOpen, color: "var(--orange)" },
  { title: "学习新技术", desc: "解释框架或库的用法", icon: Lightbulb, color: "var(--orange)" },
  { title: "UI 设计建议", desc: "提供界面设计灵感", icon: Palette, color: "var(--red)" },
  { title: "数据分析", desc: "分析数据并生成图表", icon: BarChart3, color: "var(--green)" },
  { title: "翻译润色", desc: "翻译或优化文案表达", icon: Globe, color: "var(--tint)" },
  { title: "头脑风暴", desc: "产品功能创意发散", icon: MessageSquare, color: "var(--orange)" },
  { title: "写脚本工具", desc: "自动化脚本或 CLI 工具", icon: Code2, color: "var(--green)" },
  { title: "架构设计", desc: "系统架构方案讨论", icon: PenTool, color: "var(--tint)" },
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
  const agent = agents.find((a) => a.id === "main") ?? agents[0];

  const [seed, setSeed] = useState(0);
  const cards = useMemo(() => shuffle(SUGGESTION_POOL).slice(0, 4), [seed]);
  const refreshRef = useRef<SVGSVGElement>(null);

  const handleRefresh = useCallback(() => {
    setSeed((s) => s + 1);
    if (refreshRef.current) {
      refreshRef.current.style.transition = "transform 0.5s var(--ease-out)";
      refreshRef.current.style.transform = "rotate(360deg)";
      setTimeout(() => {
        if (refreshRef.current) {
          refreshRef.current.style.transition = "none";
          refreshRef.current.style.transform = "rotate(0deg)";
        }
      }, 500);
    }
  }, []);

  return (
    <div
      className="relative flex min-h-full flex-col items-center justify-center px-8"
      style={{ animation: "scale-in var(--duration-slow) var(--ease-out)" }}
    >
      <div className="relative mb-6 text-center">
        <h2
          className="text-[26px] font-bold tracking-[-0.03em]"
          style={{
            background: "var(--gradient-border)",
            WebkitBackgroundClip: "text",
            WebkitTextFillColor: "transparent",
            backgroundClip: "text",
            animation: "fade-slide-up var(--duration-slow) var(--ease-out) 0.05s backwards",
          }}
        >
          Hi，我是{agent?.name ?? "Agent"}
          <sup
            className="ml-0.5 text-[14px] font-semibold"
            style={{ WebkitTextFillColor: "var(--tint)", color: "var(--tint)" }}
          >
            +
          </sup>
        </h2>
        {agent?.tagline && (
          <p
            className="mt-2 text-[13px]"
            style={{
              color: "var(--fill-tertiary)",
              animation: "fade-slide-up var(--duration-slow) var(--ease-out) 0.1s backwards",
            }}
          >
            {agent.tagline}
          </p>
        )}
      </div>

      <div className="relative w-full" style={{ maxWidth: 560 }}>
        <div className="grid grid-cols-2 gap-3 pb-4">
          {cards.map((card, i) => {
            const Icon = card.icon;
            return (
              <button
                key={`${card.title}-${seed}`}
                onClick={() => onPick(card.title)}
                className="group flex cursor-pointer flex-col gap-2.5 rounded-[var(--radius-md)] px-4 py-4 text-left transition-all duration-150 hover:-translate-y-0.5 active:translate-y-0 active:scale-[0.98]"
                style={{
                  backdropFilter: "saturate(180%) blur(16px)",
                  WebkitBackdropFilter: "saturate(180%) blur(16px)",
                  background: "color-mix(in srgb, var(--bg-surface) 85%, transparent)",
                  border: "0.5px solid var(--border-subtle)",
                  boxShadow: "var(--shadow-md), inset 0 1px 0 var(--highlight-top)",
                  animation: `fade-slide-up var(--duration-slow) var(--ease-out) ${0.08 + i * 0.08}s backwards`,
                }}
                onMouseEnter={(e) => { (e.currentTarget as HTMLElement).style.boxShadow = "var(--shadow-lg), var(--glow-tint-sm), inset 0 1px 0 var(--highlight-top)"; (e.currentTarget as HTMLElement).style.borderColor = "var(--border-emphasis)"; }}
                onMouseLeave={(e) => { (e.currentTarget as HTMLElement).style.boxShadow = "var(--shadow-md), inset 0 1px 0 var(--highlight-top)"; (e.currentTarget as HTMLElement).style.borderColor = "var(--border-subtle)"; }}
              >
                <div
                  className="flex h-9 w-9 items-center justify-center rounded-[var(--radius-sm)] transition-all duration-150 group-hover:scale-110 group-hover:rotate-[5deg]"
                  style={{ background: `color-mix(in srgb, ${card.color} 10%, transparent)`, color: card.color }}
                >
                  <Icon {...ICON.lg} />
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
            className="flex items-center gap-1.5 text-[12px] font-medium transition-all duration-150 hover:opacity-70 active:scale-95"
            style={{ color: "var(--fill-tertiary)" }}
          >
            <RefreshCw ref={refreshRef} {...ICON.sm} />
            换一换
          </button>
        </div>
      </div>
    </div>
  );
}
