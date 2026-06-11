import { useTranslation } from "react-i18next";
import { CheckCircle } from "@phosphor-icons/react";

export function SubStepBreadcrumb({ current, isCustom }: { current: 1 | 2 | 3; isCustom: boolean }) {
  const { t } = useTranslation("onboarding");
  const steps = isCustom
    ? [{ num: 1, label: t("step_provider") }, { num: 3, label: t("step_config") }]
    : [{ num: 1, label: t("step_provider") }, { num: 2, label: t("step_model") }, { num: 3, label: t("step_apiKey") }];

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
              {isDone ? <CheckCircle size={12} weight="bold" /> : displayNum}
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
