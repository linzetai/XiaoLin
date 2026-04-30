import { create } from "zustand";
import * as api from "../api";

export type FontSize = "small" | "standard" | "large" | "xlarge";

export const FONT_SIZE_MAP: Record<FontSize, number> = {
  small: 13,
  standard: 14,
  large: 15,
  xlarge: 16,
};

export interface DisplayConfig {
  toolCallGroupThreshold: number;
  fontSize: FontSize;
}

export interface ConfigStoreState {
  display: DisplayConfig;
  setDisplayConfig: (partial: Partial<DisplayConfig>) => void;
  loadDisplayConfig: () => Promise<void>;
}

const DEFAULT_DISPLAY: DisplayConfig = {
  toolCallGroupThreshold: 3,
  fontSize: "standard",
};

function applyFontSize(size: FontSize) {
  document.documentElement.style.fontSize = `${FONT_SIZE_MAP[size]}px`;
}

export const useConfigStore = create<ConfigStoreState>((set, get) => ({
  display: { ...DEFAULT_DISPLAY },

  setDisplayConfig: (partial) => {
    const next = { ...get().display, ...partial };
    set({ display: next });
    if (partial.fontSize) applyFontSize(partial.fontSize);
    api.setConfig("display", next).catch(() => {});
  },

  loadDisplayConfig: async () => {
    try {
      const data = await api.getConfig("display");
      const cfg = (data as { key?: string; value?: Partial<DisplayConfig> } | null);
      const val = cfg?.value ?? cfg;
      if (val && typeof val === "object") {
        const merged = { ...DEFAULT_DISPLAY, ...val as Partial<DisplayConfig> };
        set({ display: merged });
        applyFontSize(merged.fontSize);
      }
    } catch { /* use defaults */ }
  },
}));
