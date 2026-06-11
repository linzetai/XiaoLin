import { useTranslation } from "react-i18next";
import { CheckCircle, ArrowRight } from "@phosphor-icons/react";
import { ICON_SIZE } from "../../../lib/ui-tokens";

export function ModelSavedConfirmation({ model, onNext }: { model: string; onNext: () => void }) {
  const { t } = useTranslation("onboarding");
  return (
    <div className="flex flex-col items-center text-center">
      <div
        className="flex h-16 w-16 items-center justify-center rounded-full"
        style={{ background: "color-mix(in srgb, var(--green) 12%, transparent)" }}
      >
        <CheckCircle size={32} style={{ color: "var(--green)" }} />
      </div>
      <h2 className="mt-5 text-[22px] font-bold" style={{ color: "var(--fill-primary)" }}>
        {t("modelConfigDone")}
      </h2>
      <p className="mt-2 text-[14px]" style={{ color: "var(--fill-secondary)" }}>
        {t("modelReady", { model: model || t("defaultModel") })}
      </p>
      <button
        onClick={onNext}
        className="mt-8 flex cursor-pointer items-center gap-2 rounded-full px-8 py-3 text-[14px] font-medium transition-all duration-200 hover:scale-[1.02] active:scale-[0.98]"
        style={{ background: "var(--fill-primary)", color: "var(--fill-inverse)" }}
      >
        {t("exploreFeatures")} <ArrowRight size={ICON_SIZE.md} />
      </button>
    </div>
  );
}
