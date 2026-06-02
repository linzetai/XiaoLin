import { Settings } from "lucide-react";
import { ICON } from "../../../lib/ui-tokens";
import { getAllProviders } from "../../../lib/model-registry";
import type { ModelAction } from "./model-state";

export function ProviderSelectStep({ dispatch }: { dispatch: React.Dispatch<ModelAction> }) {
  const providers = getAllProviders();
  return (
    <div
      className="overflow-hidden rounded-[var(--radius-md)]"
      style={{
        background: "var(--bg-elevated)",
        border: "0.5px solid var(--separator-opaque)",
        boxShadow: "var(--shadow-md)",
      }}
    >
      <div className="p-4">
        <div className="grid grid-cols-2 gap-2.5">
          {providers.map((p) => (
            <div
              key={p.id}
              className="cursor-pointer rounded-[var(--radius-sm)] border p-3.5 transition-all hover:scale-[1.01]"
              style={{ background: "var(--bg-base)", borderColor: "var(--separator-opaque)" }}
              onClick={() => dispatch({ type: "SELECT_PROVIDER", provider: p })}
            >
              <div className="flex items-center gap-2.5">
                <span className="text-[20px] leading-none">{p.logo}</span>
                <div>
                  <div className="text-[13px] font-semibold" style={{ color: "var(--fill-primary)" }}>
                    {p.name}
                  </div>
                  <div className="mt-0.5 text-[11px]" style={{ color: "var(--fill-tertiary)" }}>
                    {p.models.length} 个模型
                  </div>
                </div>
              </div>
            </div>
          ))}
          <div
            className="flex cursor-pointer items-center justify-center gap-2 rounded-[var(--radius-sm)] border border-dashed p-3.5 transition-all hover:scale-[1.01]"
            style={{ borderColor: "var(--separator)" }}
            onClick={() => dispatch({ type: "SELECT_CUSTOM" })}
          >
            <Settings {...ICON.md} style={{ color: "var(--fill-tertiary)" }} />
            <span className="text-[13px] font-medium" style={{ color: "var(--fill-secondary)" }}>
              自定义
            </span>
          </div>
        </div>
      </div>
    </div>
  );
}
