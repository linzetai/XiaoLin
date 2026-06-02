import { CheckCircle } from "lucide-react";

export function SubStepBreadcrumb({ current, isCustom }: { current: 1 | 2 | 3; isCustom: boolean }) {
  const steps = isCustom
    ? [{ num: 1, label: "选择提供商" }, { num: 3, label: "配置信息" }]
    : [{ num: 1, label: "选择提供商" }, { num: 2, label: "选择模型" }, { num: 3, label: "配置密钥" }];

  return (
    <div className="mb-5 flex items-center justify-center gap-1">
      {steps.map((step, idx) => {
        const isDone = step.num < current;
        const isActive = step.num === current;
        const displayNum = idx + 1;
        return (
          <div key={step.num} className="flex items-center gap-1">
            <div
              className={`flex h-5 w-5 items-center justify-center rounded-full text-[10px] font-bold ${
                isDone || isActive ? "" : "opacity-30"
              }`}
              style={{
                background: isDone ? "var(--green)" : isActive ? "var(--fill-primary)" : "var(--fill-quaternary)",
                color: isDone || isActive ? (isDone ? "#fff" : "var(--fill-inverse)") : "var(--fill-inverse)",
              }}
            >
              {isDone ? <CheckCircle size={12} strokeWidth={3} /> : displayNum}
            </div>
            <span
              className={`text-[11px] ${isDone || isActive ? "" : "opacity-30"}`}
              style={{ color: isDone || isActive ? "var(--fill-primary)" : "var(--fill-tertiary)" }}
            >
              {step.label}
            </span>
            {idx < steps.length - 1 && (
              <div className="ml-1 mr-1 h-px w-4" style={{ background: "var(--separator)" }} />
            )}
          </div>
        );
      })}
    </div>
  );
}
