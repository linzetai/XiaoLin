import { CheckCircle, ArrowRight } from "lucide-react";
import { ICON } from "../../../lib/ui-tokens";

export function ModelSavedConfirmation({ model, onNext }: { model: string; onNext: () => void }) {
  return (
    <div className="flex flex-col items-center text-center">
      <div
        className="flex h-16 w-16 items-center justify-center rounded-full"
        style={{ background: "color-mix(in srgb, var(--green) 12%, transparent)" }}
      >
        <CheckCircle size={32} strokeWidth={1.5} style={{ color: "var(--green)" }} />
      </div>
      <h2 className="mt-5 text-[22px] font-bold" style={{ color: "var(--fill-primary)" }}>
        模型配置完成
      </h2>
      <p className="mt-2 text-[14px]" style={{ color: "var(--fill-secondary)" }}>
        <span className="font-medium">{model || "模型"}</span> 已就绪，接下来了解一下小林的核心功能
      </p>
      <button
        onClick={onNext}
        className="mt-8 flex cursor-pointer items-center gap-2 rounded-full px-8 py-3 text-[14px] font-medium transition-all duration-200 hover:scale-[1.02] active:scale-[0.98]"
        style={{ background: "var(--fill-primary)", color: "var(--fill-inverse)" }}
      >
        了解功能 <ArrowRight {...ICON.md} />
      </button>
    </div>
  );
}
