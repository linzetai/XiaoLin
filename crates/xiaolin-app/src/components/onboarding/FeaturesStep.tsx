import { useTranslation } from "react-i18next";
import {
  CaretLeft, Robot, ChatText, Clock, MagnifyingGlass,
  Wrench, Sparkle, ArrowRight,
} from "@phosphor-icons/react";
import { ICON_SIZE } from "../../lib/ui-tokens";

export function FeaturesStep({ onNext, onPrev }: { onNext: () => void; onPrev: () => void }) {
  const { t } = useTranslation("onboarding");

  const features = [
    { icon: Robot, cssColor: "var(--tint)", title: t("feature_multiAgent_title"), desc: t("feature_multiAgent_desc") },
    { icon: Wrench, cssColor: "var(--orange, #ED8936)", title: t("feature_tools_title"), desc: t("feature_tools_desc") },
    { icon: Clock, cssColor: "var(--purple, #B794F4)", title: t("feature_cron_title"), desc: t("feature_cron_desc") },
    { icon: MagnifyingGlass, cssColor: "var(--green)", title: t("feature_search_title"), desc: t("feature_search_desc") },
    { icon: ChatText, cssColor: "var(--blue, #63B3ED)", title: t("feature_chat_title"), desc: t("feature_chat_desc") },
    { icon: Sparkle, cssColor: "var(--yellow, #F6E05E)", title: t("feature_skills_title"), desc: t("feature_skills_desc") },
  ];

  return (
    <div className="relative">
      <div className="absolute -top-12 left-0 flex">
        <button
          onClick={onPrev}
          className="flex cursor-pointer items-center gap-1 text-[13px] font-medium transition-colors hover:opacity-80"
          style={{ color: "var(--fill-tertiary)" }}
        >
          <CaretLeft size={ICON_SIZE.md} />
          {t("back")}
        </button>
      </div>

      <div className="mb-6 text-center">
        <h2 className="text-[22px] font-bold" style={{ color: "var(--fill-primary)" }}>
          {t("featuresTitle")}
        </h2>
        <p className="mt-2 text-[13px]" style={{ color: "var(--fill-tertiary)" }}>
          {t("featuresSubtitle")}
        </p>
      </div>

      <div className="grid grid-cols-2 gap-3">
        {features.map((f) => (
          <div
            key={f.title}
            className="rounded-[var(--radius-sm)] p-4 transition-all duration-200 hover:scale-[1.01]"
            style={{ background: "var(--bg-elevated)", border: "0.5px solid var(--separator-opaque)" }}
          >
            <div
              className="mb-3 flex h-9 w-9 items-center justify-center rounded-[8px]"
              style={{ background: `color-mix(in srgb, ${f.cssColor} 10%, transparent)` }}
            >
              <f.icon size={ICON_SIZE.lg} style={{ color: f.cssColor }} />
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
          {t("getStarted")}
          <ArrowRight size={ICON_SIZE.md} />
        </button>
      </div>
    </div>
  );
}
