import { useTranslation } from "react-i18next";
import { Gear, ArrowRight } from "@phosphor-icons/react";
import { ClawIcon } from "../layout/ClawIcon";
import { ICON_SIZE } from "../../lib/ui-tokens";

export function WelcomeStep({ onNext, onImport }: { onNext: () => void; onImport: () => void }) {
  const { t } = useTranslation("onboarding");
  return (
    <div className="flex flex-col items-center text-center">
      <div style={{ animation: "scale-in var(--duration-slower) var(--ease-out)" }}>
        <ClawIcon size={72} />
      </div>
      <h1
        className="mt-6 text-[28px] font-bold tracking-tight"
        style={{ color: "var(--fill-primary)" }}
      >
        {t("welcomeTitle")}
      </h1>
      <p
        className="mt-3 max-w-[380px] text-[15px] leading-relaxed"
        style={{ color: "var(--fill-secondary)" }}
      >
        {t("welcomeDesc")}
      </p>
      <p className="mt-6 text-[13px]" style={{ color: "var(--fill-tertiary)" }}>
        {t("howToStart")}
      </p>
      <div className="mt-4 flex w-full max-w-[280px] flex-col gap-3">
        <button
          onClick={onNext}
          className="flex cursor-pointer items-center justify-center gap-2 rounded-full px-6 py-3 text-[14px] font-medium transition-all duration-200 hover:scale-[1.02] active:scale-[0.98]"
          style={{ background: "var(--fill-primary)", color: "var(--fill-inverse)" }}
        >
          {t("newUserSetup")}
          <Gear size={ICON_SIZE.md} />
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
          {t("importConfig")}
          <ArrowRight size={ICON_SIZE.md} />
        </button>
      </div>
    </div>
  );
}
