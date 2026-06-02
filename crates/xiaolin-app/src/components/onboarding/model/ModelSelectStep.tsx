import { ChevronRight, Sparkles } from "lucide-react";
import { ICON } from "../../../lib/ui-tokens";
import type { ProviderPreset } from "../../../lib/model-registry";
import type { ModelAction } from "./model-state";

export function ModelSelectStep({
  provider,
  dispatch,
}: {
  provider: ProviderPreset;
  dispatch: React.Dispatch<ModelAction>;
}) {
  return (
    <div
      className="overflow-hidden rounded-[var(--radius-md)]"
      style={{
        background: "var(--bg-elevated)",
        border: "0.5px solid var(--separator-opaque)",
        boxShadow: "var(--shadow-md)",
      }}
    >
      <div className="px-4 pb-2 pt-4">
        <div className="mb-3 flex items-center gap-2">
          <span className="text-[18px]">{provider.logo}</span>
          <span className="text-[14px] font-semibold" style={{ color: "var(--fill-primary)" }}>
            {provider.name}
          </span>
        </div>
        <div className="space-y-1.5">
          {provider.models.map((m) => (
            <div
              key={m.id}
              className="flex cursor-pointer items-center justify-between rounded-[var(--radius-sm)] px-3 py-2.5 transition-all hover:scale-[1.01]"
              style={{ background: "var(--bg-base)", border: "0.5px solid var(--separator-opaque)" }}
              onClick={() =>
                dispatch({ type: "SELECT_MODEL", modelId: m.id, contextWindow: m.contextWindow })
              }
            >
              <div>
                <div className="text-[13px] font-medium" style={{ color: "var(--fill-primary)" }}>
                  {m.name}
                </div>
                <div className="mt-0.5 text-[11px]" style={{ color: "var(--fill-tertiary)" }}>
                  {m.description}
                </div>
              </div>
              <ChevronRight {...ICON.sm} style={{ color: "var(--fill-tertiary)" }} />
            </div>
          ))}
        </div>
      </div>
      {provider.docsUrl && (
        <div
          className="flex items-center gap-2 px-4 py-2.5"
          style={{ borderTop: "0.5px solid var(--separator)" }}
        >
              <Sparkles {...ICON.sm} style={{ color: "var(--tint)" }} />
          <span className="text-[11px]" style={{ color: "var(--fill-tertiary)" }}>
            还没有 API Key？
          </span>
          <a
            href={provider.docsUrl}
            target="_blank"
            rel="noopener noreferrer"
            className="text-[11px] font-medium underline"
            style={{ color: "var(--tint)" }}
          >
            前往获取
          </a>
        </div>
      )}
    </div>
  );
}
