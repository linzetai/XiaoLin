import { ChevronDown } from "lucide-react";
import * as api from "../../lib/api";
import { SectionHeader } from "./common";

export function AgentBasicInfo({
  name,
  onNameChange,
  models,
  selectedModelValue,
  onModelSelect,
  encodeModelOption,
  effectiveOptionValue,
  effectiveModel,
  effectiveProvider,
}: {
  name: string;
  onNameChange: (v: string) => void;
  models: api.ModelInfo[];
  selectedModelValue: string;
  onModelSelect: (value: string) => void;
  encodeModelOption: (provider: string, model: string) => string;
  effectiveOptionValue: string;
  effectiveModel: string;
  effectiveProvider: string;
}) {
  return (
    <>
      <div>
        <SectionHeader>名称</SectionHeader>
        <input
          type="text"
          value={name}
          onChange={(e) => onNameChange(e.target.value)}
          className="w-full rounded-[var(--radius-sm)] px-3 py-2 text-[13px] outline-none transition-colors duration-150 focus:ring-1 focus:ring-[var(--fill-quaternary)]"
          style={{ background: "var(--bg-elevated)", color: "var(--fill-primary)", border: "0.5px solid var(--separator-opaque)" }}
        />
      </div>

      <div>
        <SectionHeader>模型</SectionHeader>
        <div className="relative">
          <select
            value={selectedModelValue}
            onChange={(e) => onModelSelect(e.target.value)}
            className="w-full cursor-pointer rounded-[var(--radius-sm)] px-3 py-2.5 pr-8 text-[13px] outline-none transition-colors duration-150 focus:ring-1 focus:ring-[var(--fill-quaternary)]"
            style={{ background: "var(--bg-elevated)", color: "var(--fill-primary)", border: "0.5px solid var(--separator-opaque)", WebkitAppearance: "none", MozAppearance: "none", appearance: "none" }}
          >
            {models.map((m) => (
              <option key={`${m.provider}/${m.model}`} value={encodeModelOption(m.provider, m.model)}>{m.model} ({m.provider})</option>
            ))}
            {!models.some((m) => encodeModelOption(m.provider, m.model) === effectiveOptionValue) && (
              <option value={effectiveOptionValue}>
                {effectiveProvider ? `${effectiveModel} (${effectiveProvider})` : effectiveModel}
              </option>
            )}
          </select>
          <ChevronDown size={12} strokeWidth={2} className="pointer-events-none absolute top-1/2 right-3 -translate-y-1/2" style={{ color: "var(--fill-tertiary)" }} />
        </div>
      </div>
    </>
  );
}
